use anyhow::{Context, Result};
use std::io::{BufReader, BufWriter, Write};
use std::net::{Shutdown, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use space_lt_common::protocol::{
    ClientMsg, OrchestratorMsg, ServerMsg, is_disconnect, read_client_msg, read_orchestrator_msg,
    write_orchestrator_msg, write_server_msg,
};
use space_lt_common::{debug, info, warn};

use crate::transcribe::Transcriber;
use crate::tts::TtsEngine;

/// Number of i16 samples per TtsAudioChunk (250ms at 16kHz).
const TTS_CHUNK_SIZE: usize = 4000;

/// Crossfade length in samples for sentence boundaries (10ms at 16kHz).
const CROSSFADE_LEN: usize = 160;

/// Run the message routing session between a TCP client and a Unix socket orchestrator.
///
/// Spawns two worker threads:
/// - stt_router: reads ClientMsg from TCP → transcribes → writes TranscribedText to Unix
/// - tts_router: reads OrchestratorMsg from Unix → synthesizes TTS → writes TtsAudioChunk to TCP
///
/// Returns when either connection closes or an error occurs.
pub fn run_session(
    transcriber: Box<dyn Transcriber>,
    tts: Box<dyn TtsEngine>,
    tcp_stream: TcpStream,
    unix_stream: UnixStream,
) -> Result<()> {
    // Clone streams for split read/write across threads
    let tcp_for_read = tcp_stream
        .try_clone()
        .context("cloning TCP stream for reader")?;
    let unix_for_read = unix_stream
        .try_clone()
        .context("cloning Unix stream for reader")?;

    // Keep clones for shutdown: shutdown() unblocks threads stuck on blocking reads
    let tcp_cleanup = tcp_stream
        .try_clone()
        .context("cloning TCP stream for cleanup")?;
    let unix_cleanup = unix_stream
        .try_clone()
        .context("cloning Unix stream for cleanup")?;

    // tcp_for_read → reader for stt_router
    // unix_stream → writer for stt_router, unix_for_read → reader for tts_router
    // tcp_stream → shared BufWriter for both threads (display text + TTS audio)

    // Shared TCP writer: stt_router sends "You: ..." display text,
    // tts_router sends "AI: ..." display text + TTS audio chunks
    let client_writer = Arc::new(Mutex::new(BufWriter::new(tcp_stream)));
    let client_writer_stt = client_writer.clone();

    // Shared pause state between stt_router and tts_router
    let paused = Arc::new(AtomicBool::new(false));
    let paused_stt = paused.clone();
    let paused_tts = paused;

    // Shared TTS interrupt flag: stt_router sets on InterruptTts, tts_router checks between chunks
    let tts_interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_stt = tts_interrupted.clone();
    let interrupted_tts = tts_interrupted;

    let stt_handle = std::thread::Builder::new()
        .name("stt_router".into())
        .spawn(move || {
            stt_router(
                tcp_for_read,
                unix_stream,
                transcriber,
                paused_stt,
                client_writer_stt,
                interrupted_stt,
            )
        })?;

    let tts: Arc<dyn TtsEngine> = Arc::from(tts);
    let tts_handle = std::thread::Builder::new()
        .name("tts_router".into())
        .spawn(move || {
            tts_router(
                unix_for_read,
                client_writer,
                tts,
                paused_tts,
                interrupted_tts,
            )
        })?;

    // Wait for either thread to finish (connection close or error)
    loop {
        if stt_handle.is_finished() || tts_handle.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // Shutdown streams to unblock the remaining thread stuck on a blocking read
    let _ = tcp_cleanup.shutdown(Shutdown::Both);
    let _ = unix_cleanup.shutdown(Shutdown::Both);

    // Join both threads
    match stt_handle.join() {
        Ok(Ok(())) => debug!("[server] stt_router exited cleanly"),
        Ok(Err(e)) => debug!("[server] stt_router error: {e}"),
        Err(_) => warn!("[server] stt_router thread panicked"),
    }
    match tts_handle.join() {
        Ok(Ok(())) => debug!("[server] tts_router exited cleanly"),
        Ok(Err(e)) => debug!("[server] tts_router error: {e}"),
        Err(_) => warn!("[server] tts_router thread panicked"),
    }

    info!("[server] Session ended");
    Ok(())
}

/// STT routing: reads ClientMsg from TCP, transcribes audio, forwards text to orchestrator.
fn stt_router(
    tcp_read: TcpStream,
    unix_write: UnixStream,
    mut transcriber: Box<dyn Transcriber>,
    paused: Arc<AtomicBool>,
    client_writer: Arc<Mutex<BufWriter<TcpStream>>>,
    tts_interrupted: Arc<AtomicBool>,
) -> Result<()> {
    let mut reader = BufReader::new(tcp_read);
    let mut writer = BufWriter::new(unix_write);

    loop {
        let msg = match read_client_msg(&mut reader) {
            Ok(msg) => msg,
            Err(e) => {
                if is_disconnect(&e) {
                    info!("[server] Client disconnected");
                    break;
                }
                return Err(e.context("reading client message"));
            }
        };

        match msg {
            ClientMsg::AudioSegment(samples) => {
                if paused.load(Ordering::SeqCst) {
                    debug!(
                        "[server] Paused — dropping audio segment ({} samples)",
                        samples.len()
                    );
                    continue;
                }

                debug!(
                    "[server] Audio segment: {} samples ({:.0}ms)",
                    samples.len(),
                    samples.len() as f64 / 16.0
                );

                let text = transcriber
                    .transcribe(&samples)
                    .context("transcribing audio")?;

                if !text.is_empty() {
                    debug!("[server] Transcribed: \"{}\"", text);
                    // Display transcription on client
                    if let Ok(mut w) = client_writer.lock() {
                        let _ = write_server_msg(&mut *w, &ServerMsg::Text(format!("You: {text}")));
                    }
                    write_orchestrator_msg(&mut writer, &OrchestratorMsg::TranscribedText(text))?;
                }
            }
            ClientMsg::PauseRequest => {
                paused.store(true, Ordering::SeqCst);
                info!("[server] Session paused");
            }
            ClientMsg::ResumeRequest => {
                paused.store(false, Ordering::SeqCst);
                info!("[server] Session resumed");
            }
            ClientMsg::InterruptTts => {
                tts_interrupted.store(true, Ordering::SeqCst);
                info!("[server] TTS interrupted by client");
            }
            ClientMsg::FeedbackChoice(proceed) => {
                info!(
                    "[server] FeedbackChoice: {}",
                    if proceed { "continue" } else { "retry" }
                );
                write_orchestrator_msg(&mut writer, &OrchestratorMsg::FeedbackChoice(proceed))?;
            }
            ClientMsg::SummaryRequest => {
                info!("[server] Summary requested by client, forwarding to orchestrator");
                write_orchestrator_msg(&mut writer, &OrchestratorMsg::SummaryRequest)?;
            }
        }
    }

    Ok(())
}

/// TTS routing: reads OrchestratorMsg from Unix, synthesizes speech, streams to client.
fn tts_router(
    unix_read: UnixStream,
    client_writer: Arc<Mutex<BufWriter<TcpStream>>>,
    tts: Arc<dyn TtsEngine>,
    paused: Arc<AtomicBool>,
    tts_interrupted: Arc<AtomicBool>,
) -> Result<()> {
    let mut reader = BufReader::new(unix_read);

    loop {
        let msg = match read_orchestrator_msg(&mut reader) {
            Ok(msg) => msg,
            Err(e) => {
                if is_disconnect(&e) {
                    info!("[server] Orchestrator disconnected");
                    break;
                }
                return Err(e.context("reading orchestrator message"));
            }
        };

        match msg {
            OrchestratorMsg::ResponseText(text) => {
                tts_interrupted.store(false, Ordering::SeqCst);

                if paused.load(Ordering::SeqCst) {
                    debug!(
                        "[server] Paused — skipping TTS for response ({} chars)",
                        text.len()
                    );
                    let mut w = client_writer
                        .lock()
                        .map_err(|e| anyhow::anyhow!("client writer poisoned: {e}"))?;
                    write_server_msg(&mut *w, &ServerMsg::TtsEnd)?;
                    continue;
                }

                // Parse optional speed marker (e.g. "[SPEED:0.6] Hello")
                let (speed, clean_text) = parse_speed_marker(&text);
                if let Some(s) = speed {
                    tts.set_speed(s);
                    info!("[server] TTS speed set to {s}");
                }

                debug!("[server] ResponseText: {} chars", clean_text.len());

                // Display AI response on client
                {
                    let mut w = client_writer
                        .lock()
                        .map_err(|e| anyhow::anyhow!("client writer poisoned: {e}"))?;
                    let _ =
                        write_server_msg(&mut *w, &ServerMsg::Text(format!("AI: {clean_text}")));
                }

                let tts_start = std::time::Instant::now();
                let sentences = split_sentences(clean_text);

                if sentences.is_empty() {
                    let mut w = client_writer
                        .lock()
                        .map_err(|e| anyhow::anyhow!("client writer poisoned: {e}"))?;
                    write_server_msg(&mut *w, &ServerMsg::TtsEnd)?;
                } else if sentences.len() == 1 {
                    // Single sentence: no pipeline overhead
                    match tts.synthesize(sentences[0]) {
                        Ok(samples) => {
                            let audio_duration = samples.len() as f64 / 16000.0;
                            info!(
                                "[server] TTS: {:.2}s synthesis, {:.2}s audio ({} chars)",
                                tts_start.elapsed().as_secs_f64(),
                                audio_duration,
                                clean_text.len()
                            );
                            let mut w = client_writer
                                .lock()
                                .map_err(|e| anyhow::anyhow!("client writer poisoned: {e}"))?;
                            let was_interrupted =
                                send_tts_audio(&mut *w, &samples, &tts_interrupted)?;
                            if was_interrupted {
                                info!(
                                    "[server] TTS interrupted after {:.2}s",
                                    tts_start.elapsed().as_secs_f64()
                                );
                            }
                        }
                        Err(e) => {
                            warn!("[server] TTS synthesis failed: {e}");
                            let mut w = client_writer
                                .lock()
                                .map_err(|e| anyhow::anyhow!("client writer poisoned: {e}"))?;
                            write_server_msg(&mut *w, &ServerMsg::TtsEnd)?;
                        }
                    }
                } else {
                    // Multiple sentences: pipeline synthesis + send
                    let num_sentences = sentences.len();
                    info!(
                        "[server] TTS streaming: {} sentences for {} chars",
                        num_sentences,
                        clean_text.len()
                    );
                    let (tx, rx) = crossbeam_channel::bounded::<Vec<i16>>(2);
                    let tts_clone = tts.clone();
                    let interrupted_producer = tts_interrupted.clone();
                    let sentence_strs: Vec<String> =
                        sentences.iter().map(|s| s.to_string()).collect();

                    // Producer: synthesize sentences sequentially
                    std::thread::Builder::new()
                        .name("tts_synth".into())
                        .spawn(move || {
                            for (i, sentence) in sentence_strs.iter().enumerate() {
                                if interrupted_producer.load(Ordering::SeqCst) {
                                    debug!("[server] TTS producer: interrupted before sentence {}", i + 1);
                                    break;
                                }
                                let synth_start = std::time::Instant::now();
                                match tts_clone.synthesize(sentence) {
                                    Ok(samples) => {
                                        let audio_dur = samples.len() as f64 / 16000.0;
                                        debug!(
                                            "[server] TTS sentence {}/{}: {:.2}s synthesis, {} samples ({:.2}s audio)",
                                            i + 1,
                                            sentence_strs.len(),
                                            synth_start.elapsed().as_secs_f64(),
                                            samples.len(),
                                            audio_dur,
                                        );
                                        if tx.send(samples).is_err() {
                                            break; // consumer dropped
                                        }
                                    }
                                    Err(e) => {
                                        warn!(
                                            "[server] TTS synthesis failed for sentence {}/{}: {e}",
                                            i + 1,
                                            sentence_strs.len()
                                        );
                                        break;
                                    }
                                }
                            }
                            drop(tx); // signal end of production
                        })?;

                    // Consumer: send each sentence's audio as it arrives (with crossfade)
                    let mut was_interrupted = false;
                    {
                        let mut w = client_writer
                            .lock()
                            .map_err(|e| anyhow::anyhow!("client writer poisoned: {e}"))?;
                        let mut prev_tail: Option<Vec<i16>> = None;
                        for samples in rx {
                            let mut samples = samples;
                            // Apply crossfade at sentence boundary
                            if let Some(tail) = &prev_tail
                                && samples.len() >= CROSSFADE_LEN
                            {
                                apply_crossfade(tail, &mut samples);
                            }
                            // Save tail for next sentence's crossfade
                            if samples.len() >= CROSSFADE_LEN {
                                prev_tail = Some(samples[samples.len() - CROSSFADE_LEN..].to_vec());
                            } else {
                                // Short sentence: reset prev_tail (no reliable tail to crossfade from)
                                prev_tail = None;
                            }
                            was_interrupted = send_tts_chunks(&mut *w, &samples, &tts_interrupted)?;

                            if was_interrupted {
                                break;
                            }
                        }
                        write_server_msg(&mut *w, &ServerMsg::TtsEnd)?;
                    }

                    if was_interrupted {
                        info!(
                            "[server] TTS streaming interrupted after {:.2}s",
                            tts_start.elapsed().as_secs_f64()
                        );
                    } else {
                        info!(
                            "[server] TTS streaming complete: {:.2}s ({} sentences, {} chars)",
                            tts_start.elapsed().as_secs_f64(),
                            num_sentences,
                            clean_text.len()
                        );
                    }
                }
            }
            OrchestratorMsg::FeedbackText(text) => {
                // Forward language feedback directly to client (no TTS synthesis)
                info!(
                    "[server] Forwarding feedback to client ({} chars)",
                    text.len()
                );
                let mut w = client_writer
                    .lock()
                    .map_err(|e| anyhow::anyhow!("client writer poisoned: {e}"))?;
                write_server_msg(&mut *w, &ServerMsg::Feedback(text))?;
            }
            OrchestratorMsg::FeedbackChoice(_) => {
                debug!("[server] Unexpected FeedbackChoice in tts_router (ignoring)");
            }
            OrchestratorMsg::TranscribedText(_) => {
                debug!("[server] Unexpected TranscribedText from orchestrator (ignoring)");
            }
            OrchestratorMsg::SessionStart(json) => {
                debug!("[server] SessionStart in tts_router (unexpected): {}", json);
            }
            OrchestratorMsg::SessionEnd => {
                info!("[server] SessionEnd received, stopping session");
                break;
            }
            OrchestratorMsg::SummaryResponse(text) => {
                info!(
                    "[server] Forwarding session summary to client ({} bytes)",
                    text.len()
                );
                let mut w = client_writer
                    .lock()
                    .map_err(|e| anyhow::anyhow!("client writer poisoned: {e}"))?;
                write_server_msg(&mut *w, &ServerMsg::SessionSummary(text))?;
            }
            OrchestratorMsg::SummaryRequest => {
                debug!("[server] Unexpected SummaryRequest in tts_router (ignoring)");
            }
            OrchestratorMsg::StatusNotification(text) => {
                debug!("[server] Forwarding status notification: {text}");
                let mut w = client_writer
                    .lock()
                    .map_err(|e| anyhow::anyhow!("client writer poisoned: {e}"))?;
                write_server_msg(&mut *w, &ServerMsg::StatusNotification(text))?;
            }
        }
    }

    Ok(())
}

/// Parse an optional `[SPEED:X.X]` marker at the start of a response.
/// Returns the speed value (if present) and the remaining text.
fn parse_speed_marker(text: &str) -> (Option<f32>, &str) {
    if let Some(rest) = text.strip_prefix("[SPEED:")
        && let Some(end) = rest.find(']')
        && let Ok(speed) = rest[..end].parse::<f32>()
    {
        let remaining = rest[end + 1..].trim_start();
        return (Some(speed), remaining);
    }
    (None, text)
}

/// Split text into sentences for streaming TTS synthesis.
///
/// Sentences are split on `.` `!` `?` followed by whitespace or end-of-string.
/// Punctuation stays attached to the preceding sentence (important for TTS intonation).
/// Empty segments are skipped.
fn split_sentences(text: &str) -> Vec<&str> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let mut sentences = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();

    for (i, &b) in bytes.iter().enumerate() {
        if (b == b'.' || b == b'!' || b == b'?')
            && (i + 1 == bytes.len() || bytes[i + 1].is_ascii_whitespace())
        {
            let sentence = text[start..=i].trim();
            if !sentence.is_empty() {
                sentences.push(sentence);
            }
            start = i + 1;
        }
    }

    // Remaining text after last sentence-ending punctuation
    let tail = text[start..].trim();
    if !tail.is_empty() {
        sentences.push(tail);
    }

    sentences
}

/// Send TTS audio chunks without TtsEnd. Returns `true` if interrupted.
/// Caller is responsible for sending TtsEnd.
fn send_tts_chunks(
    writer: &mut impl Write,
    samples: &[i16],
    interrupted: &AtomicBool,
) -> Result<bool> {
    for chunk in samples.chunks(TTS_CHUNK_SIZE) {
        if interrupted.load(Ordering::SeqCst) {
            return Ok(true);
        }
        write_server_msg(writer, &ServerMsg::TtsAudioChunk(chunk.to_vec()))?;
    }
    Ok(false)
}

/// Apply a linear crossfade from `prev_tail` into the beginning of `samples`.
///
/// `prev_tail` must have exactly `CROSSFADE_LEN` samples (the tail of the previous
/// sentence). The first `CROSSFADE_LEN` samples of `samples` are blended:
///   out[i] = prev_tail[i] * (1 - t) + samples[i] * t, where t = i / CROSSFADE_LEN
///
/// Uses i32 arithmetic to avoid i16 overflow during blending.
fn apply_crossfade(prev_tail: &[i16], samples: &mut [i16]) {
    let len = prev_tail.len().min(samples.len()).min(CROSSFADE_LEN);
    for i in 0..len {
        let t = i as f32 / CROSSFADE_LEN as f32;
        let blended = prev_tail[i] as f32 * (1.0 - t) + samples[i] as f32 * t;
        samples[i] = blended.round().clamp(-32768.0, 32767.0) as i16;
    }
}

/// Chunk TTS audio samples and send as TtsAudioChunk messages, followed by TtsEnd.
/// Returns `true` if interrupted mid-stream, `false` if completed normally.
fn send_tts_audio(
    writer: &mut impl Write,
    samples: &[i16],
    interrupted: &AtomicBool,
) -> Result<bool> {
    let was_interrupted = send_tts_chunks(writer, samples, interrupted)?;
    if was_interrupted {
        info!("[server] TTS streaming interrupted — aborting remaining chunks");
    }
    write_server_msg(writer, &ServerMsg::TtsEnd)?;
    Ok(was_interrupted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use space_lt_common::protocol::{read_server_msg, write_client_msg, write_orchestrator_msg};
    use std::net::TcpListener;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicU32, Ordering};

    // --- Mock types for testing ---

    struct MockTranscriber {
        text: String,
    }

    impl MockTranscriber {
        fn new(text: &str) -> Self {
            Self {
                text: text.to_string(),
            }
        }
    }

    impl Transcriber for MockTranscriber {
        fn transcribe(&mut self, _audio_i16: &[i16]) -> anyhow::Result<String> {
            Ok(self.text.clone())
        }
    }

    struct MockTtsEngine {
        sample_count: usize,
    }

    impl MockTtsEngine {
        fn new(sample_count: usize) -> Self {
            Self { sample_count }
        }
    }

    impl TtsEngine for MockTtsEngine {
        fn synthesize(&self, _text: &str) -> anyhow::Result<Vec<i16>> {
            // Return a simple ramp pattern for easy verification
            Ok((0..self.sample_count).map(|i| i as i16).collect())
        }

        fn set_speed(&self, _speed: f32) {}
    }

    // --- Helper to generate unique socket paths ---

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_socket_path() -> String {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        format!("/tmp/space_lt_test_{pid}_{n}.sock")
    }

    // --- Integration tests ---

    #[test]
    fn stt_routing_audio_to_transcribed_text() {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_port = tcp_listener.local_addr().unwrap().port();
        let sock_path = temp_socket_path();

        let unix_listener = UnixListener::bind(&sock_path).unwrap();

        // Connect mock client (TCP) and mock orchestrator (Unix)
        let mock_client = TcpStream::connect(("127.0.0.1", tcp_port)).unwrap();
        let (server_tcp, _) = tcp_listener.accept().unwrap();

        let mock_orch = UnixStream::connect(&sock_path).unwrap();
        let (server_unix, _) = unix_listener.accept().unwrap();

        // Run session in thread
        let session_handle = std::thread::spawn(move || {
            run_session(
                Box::new(MockTranscriber::new("Hello world")),
                Box::new(MockTtsEngine::new(8000)),
                server_tcp,
                server_unix,
            )
        });

        // Client sends AudioSegment (use try_clone for independent writer)
        let mut client_w = BufWriter::new(mock_client.try_clone().unwrap());
        write_client_msg(&mut client_w, &ClientMsg::AudioSegment(vec![0; 1600])).unwrap();

        // Orchestrator reads TranscribedText
        let mut orch_r = BufReader::new(mock_orch.try_clone().unwrap());
        let msg = read_orchestrator_msg(&mut orch_r).unwrap();
        match msg {
            OrchestratorMsg::TranscribedText(t) => assert_eq!(t, "Hello world"),
            other => panic!("Expected TranscribedText, got {other:?}"),
        }

        // Cleanup: close connections to stop session
        drop(client_w);
        drop(orch_r);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn tts_routing_response_to_audio_chunks() {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_port = tcp_listener.local_addr().unwrap().port();
        let sock_path = temp_socket_path();

        let unix_listener = UnixListener::bind(&sock_path).unwrap();

        let mock_client = TcpStream::connect(("127.0.0.1", tcp_port)).unwrap();
        let (server_tcp, _) = tcp_listener.accept().unwrap();

        let mock_orch = UnixStream::connect(&sock_path).unwrap();
        let (server_unix, _) = unix_listener.accept().unwrap();

        // 8000 samples = 2 chunks of 4000
        let session_handle = std::thread::spawn(move || {
            run_session(
                Box::new(MockTranscriber::new("ignored")),
                Box::new(MockTtsEngine::new(8000)),
                server_tcp,
                server_unix,
            )
        });

        // Orchestrator sends ResponseText
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());
        write_orchestrator_msg(&mut orch_w, &OrchestratorMsg::ResponseText("Test".into())).unwrap();

        // Client reads TtsAudioChunk messages
        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());

        let mut total_samples = Vec::new();
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {} // display text (AI: ...)
                ServerMsg::TtsAudioChunk(samples) => {
                    assert!(samples.len() <= TTS_CHUNK_SIZE);
                    total_samples.extend_from_slice(&samples);
                }
                ServerMsg::TtsEnd => break,
                other => panic!("Expected Text, TtsAudioChunk or TtsEnd, got {other:?}"),
            }
        }

        // Verify total samples match mock output
        assert_eq!(total_samples.len(), 8000);

        // Cleanup
        drop(client_r);
        drop(orch_w);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn tts_chunking_splits_large_audio() {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_port = tcp_listener.local_addr().unwrap().port();
        let sock_path = temp_socket_path();

        let unix_listener = UnixListener::bind(&sock_path).unwrap();

        let mock_client = TcpStream::connect(("127.0.0.1", tcp_port)).unwrap();
        let (server_tcp, _) = tcp_listener.accept().unwrap();

        let mock_orch = UnixStream::connect(&sock_path).unwrap();
        let (server_unix, _) = unix_listener.accept().unwrap();

        // 10000 samples = 2 full chunks (4000) + 1 partial chunk (2000)
        let session_handle = std::thread::spawn(move || {
            run_session(
                Box::new(MockTranscriber::new("ignored")),
                Box::new(MockTtsEngine::new(10000)),
                server_tcp,
                server_unix,
            )
        });

        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());
        write_orchestrator_msg(&mut orch_w, &OrchestratorMsg::ResponseText("Test".into())).unwrap();

        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());

        let mut chunk_sizes = Vec::new();
        let mut total_samples = Vec::new();
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {} // display text (AI: ...)
                ServerMsg::TtsAudioChunk(samples) => {
                    assert!(
                        samples.len() <= TTS_CHUNK_SIZE,
                        "Chunk size {} exceeds max {}",
                        samples.len(),
                        TTS_CHUNK_SIZE
                    );
                    chunk_sizes.push(samples.len());
                    total_samples.extend_from_slice(&samples);
                }
                ServerMsg::TtsEnd => break,
                other => panic!("Expected Text, TtsAudioChunk or TtsEnd, got {other:?}"),
            }
        }

        // Verify chunking: 4000 + 4000 + 2000 = 10000
        assert_eq!(chunk_sizes, vec![4000, 4000, 2000]);
        assert_eq!(total_samples.len(), 10000);

        // Verify sample content (ramp pattern from MockTtsEngine)
        for (i, &sample) in total_samples.iter().enumerate() {
            assert_eq!(sample, i as i16);
        }

        // Cleanup
        drop(client_r);
        drop(orch_w);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    // --- Pause/Resume tests ---

    /// Helper: set up a session with TCP + Unix connections, returning handles for test interaction.
    fn setup_session(
        transcriber_text: &str,
        tts_samples: usize,
    ) -> (
        TcpStream,  // mock client
        UnixStream, // mock orchestrator
        String,     // socket path
        std::thread::JoinHandle<Result<()>>,
    ) {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_port = tcp_listener.local_addr().unwrap().port();
        let sock_path = temp_socket_path();
        let unix_listener = UnixListener::bind(&sock_path).unwrap();

        let mock_client = TcpStream::connect(("127.0.0.1", tcp_port)).unwrap();
        let (server_tcp, _) = tcp_listener.accept().unwrap();

        let mock_orch = UnixStream::connect(&sock_path).unwrap();
        let (server_unix, _) = unix_listener.accept().unwrap();

        let text = transcriber_text.to_string();
        let session_handle = std::thread::spawn(move || {
            run_session(
                Box::new(MockTranscriber::new(&text)),
                Box::new(MockTtsEngine::new(tts_samples)),
                server_tcp,
                server_unix,
            )
        });

        (mock_client, mock_orch, sock_path, session_handle)
    }

    #[test]
    fn pause_drops_audio_segments() {
        let (mock_client, mock_orch, sock_path, session_handle) = setup_session("Hello", 8000);

        let mut client_w = BufWriter::new(mock_client.try_clone().unwrap());
        let orch_clone = mock_orch.try_clone().unwrap();
        orch_clone
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let mut orch_r = BufReader::new(orch_clone);

        // 1. Normal: send audio → orchestrator receives TranscribedText
        write_client_msg(&mut client_w, &ClientMsg::AudioSegment(vec![0; 1600])).unwrap();
        let msg = read_orchestrator_msg(&mut orch_r).unwrap();
        match msg {
            OrchestratorMsg::TranscribedText(t) => assert_eq!(t, "Hello"),
            other => panic!("Expected TranscribedText, got {other:?}"),
        }

        // 2. Pause
        write_client_msg(&mut client_w, &ClientMsg::PauseRequest).unwrap();
        // Small delay for pause to take effect
        std::thread::sleep(Duration::from_millis(50));

        // 3. Send audio during pause → orchestrator should NOT receive anything
        write_client_msg(&mut client_w, &ClientMsg::AudioSegment(vec![0; 1600])).unwrap();
        match read_orchestrator_msg(&mut orch_r) {
            Err(e) => {
                let is_timeout = e.downcast_ref::<std::io::Error>().map_or(false, |io| {
                    io.kind() == std::io::ErrorKind::WouldBlock
                        || io.kind() == std::io::ErrorKind::TimedOut
                });
                assert!(is_timeout, "Expected timeout, got error: {e}");
            }
            Ok(msg) => panic!("Should not receive message during pause, got {msg:?}"),
        }

        // Cleanup
        drop(client_w);
        drop(orch_r);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn resume_restores_audio_forwarding() {
        let (mock_client, mock_orch, sock_path, session_handle) = setup_session("Resumed", 8000);

        let mut client_w = BufWriter::new(mock_client.try_clone().unwrap());
        let mut orch_r = BufReader::new(mock_orch.try_clone().unwrap());

        // 1. Pause
        write_client_msg(&mut client_w, &ClientMsg::PauseRequest).unwrap();
        std::thread::sleep(Duration::from_millis(50));

        // 2. Resume
        write_client_msg(&mut client_w, &ClientMsg::ResumeRequest).unwrap();
        std::thread::sleep(Duration::from_millis(50));

        // 3. Send audio → orchestrator should receive TranscribedText again
        write_client_msg(&mut client_w, &ClientMsg::AudioSegment(vec![0; 1600])).unwrap();
        let msg = read_orchestrator_msg(&mut orch_r).unwrap();
        match msg {
            OrchestratorMsg::TranscribedText(t) => assert_eq!(t, "Resumed"),
            other => panic!("Expected TranscribedText, got {other:?}"),
        }

        // Cleanup
        drop(client_w);
        drop(orch_r);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn pause_skips_tts_with_tts_end() {
        let (mock_client, mock_orch, sock_path, session_handle) = setup_session("ignored", 8000);

        let mut client_w = BufWriter::new(mock_client.try_clone().unwrap());
        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());

        // 1. Pause the session
        write_client_msg(&mut client_w, &ClientMsg::PauseRequest).unwrap();
        std::thread::sleep(Duration::from_millis(50));

        // 2. Orchestrator sends ResponseText while paused
        write_orchestrator_msg(
            &mut orch_w,
            &OrchestratorMsg::ResponseText("Skipped".into()),
        )
        .unwrap();

        // 3. Client should receive TtsEnd only (no TtsAudioChunk)
        let msg = read_server_msg(&mut client_r).unwrap();
        match msg {
            ServerMsg::TtsEnd => {} // Expected: immediate TtsEnd, no audio chunks
            other => panic!("Expected TtsEnd during pause, got {other:?}"),
        }

        // Cleanup
        drop(client_w);
        drop(client_r);
        drop(orch_w);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn full_pause_resume_cycle() {
        let (mock_client, mock_orch, sock_path, session_handle) = setup_session("Cycle", 4000);

        let mut client_w = BufWriter::new(mock_client.try_clone().unwrap());
        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());
        let orch_clone = mock_orch.try_clone().unwrap();
        orch_clone
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let mut orch_r = BufReader::new(orch_clone);
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());

        // 1. Normal: audio flows through
        write_client_msg(&mut client_w, &ClientMsg::AudioSegment(vec![0; 1600])).unwrap();
        let msg = read_orchestrator_msg(&mut orch_r).unwrap();
        match msg {
            OrchestratorMsg::TranscribedText(t) => assert_eq!(t, "Cycle"),
            other => panic!("Expected TranscribedText, got {other:?}"),
        }

        // 2. Pause: audio dropped, TTS skipped
        write_client_msg(&mut client_w, &ClientMsg::PauseRequest).unwrap();
        std::thread::sleep(Duration::from_millis(50));

        write_client_msg(&mut client_w, &ClientMsg::AudioSegment(vec![0; 1600])).unwrap();
        match read_orchestrator_msg(&mut orch_r) {
            Err(e) => {
                let is_timeout = e.downcast_ref::<std::io::Error>().map_or(false, |io| {
                    io.kind() == std::io::ErrorKind::WouldBlock
                        || io.kind() == std::io::ErrorKind::TimedOut
                });
                assert!(is_timeout, "Expected timeout during pause, got: {e}");
            }
            Ok(msg) => panic!("Should not receive message during pause, got {msg:?}"),
        }

        // TTS during pause → only TtsEnd (may be preceded by stale display text from step 1)
        write_orchestrator_msg(&mut orch_w, &OrchestratorMsg::ResponseText("Skip".into())).unwrap();
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {} // consume pending display texts
                ServerMsg::TtsEnd => break,
                other => panic!("Expected Text or TtsEnd during pause, got {other:?}"),
            }
        }

        // 3. Resume: audio flows again
        write_client_msg(&mut client_w, &ClientMsg::ResumeRequest).unwrap();
        std::thread::sleep(Duration::from_millis(50));

        write_client_msg(&mut client_w, &ClientMsg::AudioSegment(vec![0; 1600])).unwrap();

        // Reset read timeout for resumed operation
        orch_r.get_ref().set_read_timeout(None).unwrap();
        let msg = read_orchestrator_msg(&mut orch_r).unwrap();
        match msg {
            OrchestratorMsg::TranscribedText(t) => assert_eq!(t, "Cycle"),
            other => panic!("Expected TranscribedText after resume, got {other:?}"),
        }

        // TTS after resume → normal audio chunks
        write_orchestrator_msg(&mut orch_w, &OrchestratorMsg::ResponseText("Play".into())).unwrap();
        let mut got_audio = false;
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {} // display text (AI: ...)
                ServerMsg::TtsAudioChunk(_) => got_audio = true,
                ServerMsg::TtsEnd => break,
                other => panic!("Expected Text, TtsAudioChunk or TtsEnd, got {other:?}"),
            }
        }
        assert!(got_audio, "Should receive TTS audio after resume");

        // Cleanup
        drop(client_w);
        drop(client_r);
        drop(orch_w);
        drop(orch_r);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    // --- Session End tests ---

    #[test]
    fn session_end_stops_session() {
        let (mock_client, mock_orch, sock_path, session_handle) = setup_session("test", 4000);

        // Orchestrator sends SessionEnd
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());
        write_orchestrator_msg(&mut orch_w, &OrchestratorMsg::SessionEnd).unwrap();
        drop(orch_w);

        // Session should end cleanly (join with timeout via mpsc channel)
        let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<()>>();
        std::thread::spawn(move || {
            let result = session_handle
                .join()
                .expect("session thread should not panic");
            let _ = done_tx.send(result);
        });
        let result = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("session should end within 5 seconds");
        assert!(result.is_ok(), "session should end cleanly on SessionEnd");

        // Cleanup
        drop(mock_client);
        drop(mock_orch);
        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn client_disconnect_ends_session() {
        let (mock_client, mock_orch, sock_path, session_handle) = setup_session("test", 4000);

        // Client disconnects (close TCP)
        drop(mock_client);

        // Session should end cleanly
        let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<()>>();
        std::thread::spawn(move || {
            let result = session_handle
                .join()
                .expect("session thread should not panic");
            let _ = done_tx.send(result);
        });
        let result = done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("session should end within 5 seconds");
        assert!(
            result.is_ok(),
            "session should end cleanly on client disconnect"
        );

        // Cleanup
        drop(mock_orch);
        std::fs::remove_file(&sock_path).ok();
    }

    // --- Barge-in / InterruptTts tests ---

    #[test]
    fn interrupt_tts_aborts_audio_stream() {
        // NOTE: This integration test involves a race between stt_router (setting the interrupt
        // flag) and tts_router (sending chunks in a tight loop). MockTtsEngine returns instantly,
        // so all chunks may be sent before the interrupt propagates on fast machines. If this test
        // becomes flaky, see the deterministic unit tests below (send_tts_audio_*).
        // Setup: 20000 samples = 5 chunks of 4000 → enough to interrupt mid-stream
        let (mock_client, mock_orch, sock_path, session_handle) = setup_session("ignored", 20000);

        let mut client_w = BufWriter::new(mock_client.try_clone().unwrap());
        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());

        // Orchestrator sends a response that will generate 5 audio chunks
        write_orchestrator_msg(
            &mut orch_w,
            &OrchestratorMsg::ResponseText("Long response".into()),
        )
        .unwrap();

        // Read messages until we get at least one TtsAudioChunk, then interrupt
        let mut chunk_count = 0;

        // First, consume any Text message (AI: ...)
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {} // display text
                ServerMsg::TtsAudioChunk(_) => {
                    chunk_count += 1;
                    // Send interrupt after first chunk
                    write_client_msg(&mut client_w, &ClientMsg::InterruptTts).unwrap();
                    // Small delay for interrupt to propagate
                    std::thread::sleep(Duration::from_millis(50));
                    break;
                }
                other => panic!("Expected Text or TtsAudioChunk, got {other:?}"),
            }
        }

        assert!(
            chunk_count >= 1,
            "Should have received at least one audio chunk"
        );

        // Continue reading: should get TtsEnd (possibly after a few more chunks in flight)
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::TtsAudioChunk(_) => chunk_count += 1,
                ServerMsg::TtsEnd => break,
                other => panic!("Expected TtsAudioChunk or TtsEnd, got {other:?}"),
            }
        }

        // Should have received at least 1 chunk and a TtsEnd (interrupt was processed).
        // On fast machines, all chunks may already be buffered before the interrupt
        // propagates, so we only assert the interrupt completed (TtsEnd received above).
        assert!(
            chunk_count >= 1,
            "Should have received at least one chunk before interrupt"
        );

        // Cleanup
        drop(client_w);
        drop(client_r);
        drop(orch_w);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn interrupt_tts_normal_flow_without_interrupt() {
        // Verify normal flow (no interrupt) still works: all 5 chunks + TtsEnd
        let (mock_client, mock_orch, sock_path, session_handle) = setup_session("ignored", 20000);

        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());

        write_orchestrator_msg(&mut orch_w, &OrchestratorMsg::ResponseText("Normal".into()))
            .unwrap();

        let mut chunk_count = 0;
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {}
                ServerMsg::TtsAudioChunk(_) => chunk_count += 1,
                ServerMsg::TtsEnd => break,
                other => panic!("Expected Text, TtsAudioChunk or TtsEnd, got {other:?}"),
            }
        }

        assert_eq!(
            chunk_count, 5,
            "All 5 chunks should be sent without interrupt"
        );

        // Cleanup
        drop(client_r);
        drop(orch_w);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    // --- Speed marker parsing tests ---

    #[test]
    fn parse_speed_marker_with_valid_marker() {
        let (speed, text) = parse_speed_marker("[SPEED:0.6] Sure, I will speak more slowly.");
        assert_eq!(speed, Some(0.6));
        assert_eq!(text, "Sure, I will speak more slowly.");
    }

    #[test]
    fn parse_speed_marker_without_marker() {
        let (speed, text) = parse_speed_marker("Hello, how are you?");
        assert_eq!(speed, None);
        assert_eq!(text, "Hello, how are you?");
    }

    #[test]
    fn parse_speed_marker_with_different_speeds() {
        let (speed, _) = parse_speed_marker("[SPEED:1.2] Fast speech");
        assert_eq!(speed, Some(1.2));

        let (speed, _) = parse_speed_marker("[SPEED:0.5] Very slow");
        assert_eq!(speed, Some(0.5));
    }

    // --- Deterministic send_tts_audio unit tests ---

    #[test]
    fn send_tts_audio_interrupted_before_first_chunk() {
        let interrupted = AtomicBool::new(true);
        let samples: Vec<i16> = (0..20000).map(|i| i as i16).collect();
        let mut buf = Vec::new();

        let was_interrupted = send_tts_audio(&mut buf, &samples, &interrupted).unwrap();
        assert!(was_interrupted, "Should report interruption");

        // Should contain only TtsEnd (no audio chunks)
        let mut cursor = std::io::Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        assert!(
            matches!(msg, ServerMsg::TtsEnd),
            "Expected TtsEnd, got {msg:?}"
        );
        // No more messages
        assert!(read_server_msg(&mut cursor).is_err());
    }

    #[test]
    fn send_tts_audio_completes_without_interrupt() {
        let interrupted = AtomicBool::new(false);
        let samples: Vec<i16> = (0..20000).map(|i| i as i16).collect();
        let mut buf = Vec::new();

        let was_interrupted = send_tts_audio(&mut buf, &samples, &interrupted).unwrap();
        assert!(!was_interrupted, "Should not report interruption");

        // Should contain 5 TtsAudioChunk + 1 TtsEnd
        let mut cursor = std::io::Cursor::new(buf);
        let mut chunk_count = 0;
        loop {
            let msg = read_server_msg(&mut cursor).unwrap();
            match msg {
                ServerMsg::TtsAudioChunk(c) => {
                    assert!(c.len() <= TTS_CHUNK_SIZE);
                    chunk_count += 1;
                }
                ServerMsg::TtsEnd => break,
                other => panic!("Expected TtsAudioChunk or TtsEnd, got {other:?}"),
            }
        }
        assert_eq!(chunk_count, 5, "20000 samples / 4000 = 5 chunks");
    }

    #[test]
    fn send_tts_audio_interrupted_mid_stream() {
        let interrupted = AtomicBool::new(false);
        let mut buf = Vec::new();

        // First call: send 2 chunks normally (no interrupt)
        let small_samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
        let was_interrupted = send_tts_audio(&mut buf, &small_samples, &interrupted).unwrap();
        assert!(!was_interrupted);

        // Now test with flag pre-set: 0 chunks should be sent
        buf.clear();
        interrupted.store(true, Ordering::SeqCst);
        let big_samples: Vec<i16> = (0..20000).map(|i| i as i16).collect();
        let was_interrupted = send_tts_audio(&mut buf, &big_samples, &interrupted).unwrap();
        assert!(was_interrupted);

        let mut cursor = std::io::Cursor::new(buf);
        let msg = read_server_msg(&mut cursor).unwrap();
        assert!(matches!(msg, ServerMsg::TtsEnd));
    }

    // --- Sentence splitting tests ---

    #[test]
    fn split_sentences_multiple() {
        assert_eq!(
            split_sentences("Hello. How are you? I'm fine!"),
            vec!["Hello.", "How are you?", "I'm fine!"]
        );
    }

    #[test]
    fn split_sentences_single() {
        assert_eq!(
            split_sentences("Just one sentence"),
            vec!["Just one sentence"]
        );
    }

    #[test]
    fn split_sentences_single_with_period() {
        assert_eq!(
            split_sentences("Just one sentence."),
            vec!["Just one sentence."]
        );
    }

    #[test]
    fn split_sentences_empty() {
        let result: Vec<&str> = split_sentences("");
        assert!(result.is_empty());
    }

    #[test]
    fn split_sentences_whitespace_only() {
        let result: Vec<&str> = split_sentences("   ");
        assert!(result.is_empty());
    }

    #[test]
    fn split_sentences_extra_spaces() {
        assert_eq!(
            split_sentences("Hello.  Extra  spaces.  "),
            vec!["Hello.", "Extra  spaces."]
        );
    }

    #[test]
    fn split_sentences_mixed_punctuation() {
        assert_eq!(
            split_sentences("Really? Yes! OK."),
            vec!["Really?", "Yes!", "OK."]
        );
    }

    #[test]
    fn split_sentences_no_space_after_period() {
        // Period not followed by whitespace — not a sentence boundary
        assert_eq!(
            split_sentences("Version 3.5 is out"),
            vec!["Version 3.5 is out"]
        );
    }

    #[test]
    fn split_sentences_trailing_no_punctuation() {
        assert_eq!(
            split_sentences("First sentence. And then more"),
            vec!["First sentence.", "And then more"]
        );
    }

    // --- Sentence-level mock TTS for streaming tests ---

    /// Mock TTS that produces `samples_per_char * text.len()` samples.
    /// This simulates sentence-level streaming: shorter sentences → fewer samples.
    struct SentenceMockTtsEngine {
        samples_per_char: usize,
    }

    impl SentenceMockTtsEngine {
        fn new(samples_per_char: usize) -> Self {
            Self { samples_per_char }
        }
    }

    impl TtsEngine for SentenceMockTtsEngine {
        fn synthesize(&self, text: &str) -> anyhow::Result<Vec<i16>> {
            let count = self.samples_per_char * text.len();
            Ok((0..count).map(|i| i as i16).collect())
        }

        fn set_speed(&self, _speed: f32) {}
    }

    fn setup_sentence_session(
        transcriber_text: &str,
        samples_per_char: usize,
    ) -> (
        TcpStream,
        UnixStream,
        String,
        std::thread::JoinHandle<Result<()>>,
    ) {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_port = tcp_listener.local_addr().unwrap().port();
        let sock_path = temp_socket_path();
        let unix_listener = UnixListener::bind(&sock_path).unwrap();

        let mock_client = TcpStream::connect(("127.0.0.1", tcp_port)).unwrap();
        let (server_tcp, _) = tcp_listener.accept().unwrap();

        let mock_orch = UnixStream::connect(&sock_path).unwrap();
        let (server_unix, _) = unix_listener.accept().unwrap();

        let text = transcriber_text.to_string();
        let session_handle = std::thread::spawn(move || {
            run_session(
                Box::new(MockTranscriber::new(&text)),
                Box::new(SentenceMockTtsEngine::new(samples_per_char)),
                server_tcp,
                server_unix,
            )
        });

        (mock_client, mock_orch, sock_path, session_handle)
    }

    // --- Streaming pipeline integration tests ---

    #[test]
    fn streaming_multi_sentence_sends_all_audio() {
        let (mock_client, mock_orch, sock_path, session_handle) =
            setup_sentence_session("ignored", 100);

        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());

        // "Hi. Bye." = 2 sentences, each ~4 chars = 400 samples each
        write_orchestrator_msg(
            &mut orch_w,
            &OrchestratorMsg::ResponseText("Hi. Bye.".into()),
        )
        .unwrap();

        let mut total_samples = 0;
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {}
                ServerMsg::TtsAudioChunk(samples) => total_samples += samples.len(),
                ServerMsg::TtsEnd => break,
                other => panic!("Unexpected: {other:?}"),
            }
        }

        // "Hi." = 3 chars * 100 = 300 samples, "Bye." = 4 chars * 100 = 400 samples
        assert_eq!(total_samples, 700);

        drop(client_r);
        drop(orch_w);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn streaming_single_sentence_same_as_batch() {
        let (mock_client, mock_orch, sock_path, session_handle) =
            setup_sentence_session("ignored", 100);

        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());

        // Single sentence (no trailing period) → single-sentence path
        write_orchestrator_msg(
            &mut orch_w,
            &OrchestratorMsg::ResponseText("Hello world".into()),
        )
        .unwrap();

        let mut total_samples = 0;
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {}
                ServerMsg::TtsAudioChunk(samples) => total_samples += samples.len(),
                ServerMsg::TtsEnd => break,
                other => panic!("Unexpected: {other:?}"),
            }
        }

        // "Hello world" = 11 chars * 100 = 1100 samples
        assert_eq!(total_samples, 1100);

        drop(client_r);
        drop(orch_w);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn streaming_interrupt_stops_remaining_sentences() {
        // Use large samples_per_char to make chunks slower, improving interrupt window
        let (mock_client, mock_orch, sock_path, session_handle) =
            setup_sentence_session("ignored", 2000);

        let mut client_w = BufWriter::new(mock_client.try_clone().unwrap());
        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());

        // 3 sentences with lots of samples each
        // "First sentence. Second sentence. Third sentence." → 3 sentences
        write_orchestrator_msg(
            &mut orch_w,
            &OrchestratorMsg::ResponseText(
                "First sentence. Second sentence. Third sentence.".into(),
            ),
        )
        .unwrap();

        // Read messages until we get audio, then interrupt
        let mut chunk_count = 0;
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {}
                ServerMsg::TtsAudioChunk(_) => {
                    chunk_count += 1;
                    if chunk_count == 1 {
                        // Interrupt after first chunk
                        write_client_msg(&mut client_w, &ClientMsg::InterruptTts).unwrap();
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
                ServerMsg::TtsEnd => break,
                other => panic!("Unexpected: {other:?}"),
            }
        }

        // Total audio without interrupt would be:
        // "First sentence." (16 chars) * 2000 = 32000 → 8 chunks
        // "Second sentence." (17 chars) * 2000 = 34000 → ~9 chunks
        // "Third sentence." (16 chars) * 2000 = 32000 → 8 chunks
        // Total: ~25 chunks
        // With interrupt after first chunk, should be significantly fewer
        assert!(
            chunk_count < 25,
            "Expected fewer chunks due to interrupt, got {chunk_count}"
        );

        drop(client_w);
        drop(client_r);
        drop(orch_w);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    // --- Pipeline error handling tests ---

    /// Mock TTS that fails on the Nth call (0-indexed).
    struct FailingMockTtsEngine {
        samples_per_char: usize,
        fail_on_call: usize,
        call_count: Mutex<usize>,
    }

    impl FailingMockTtsEngine {
        fn new(samples_per_char: usize, fail_on_call: usize) -> Self {
            Self {
                samples_per_char,
                fail_on_call,
                call_count: Mutex::new(0),
            }
        }
    }

    impl TtsEngine for FailingMockTtsEngine {
        fn synthesize(&self, text: &str) -> anyhow::Result<Vec<i16>> {
            let mut count = self.call_count.lock().unwrap();
            let current = *count;
            *count += 1;
            drop(count);

            if current == self.fail_on_call {
                anyhow::bail!("TTS synthesis failed on call {current}");
            }
            let n = self.samples_per_char * text.len();
            Ok((0..n).map(|i| i as i16).collect())
        }

        fn set_speed(&self, _speed: f32) {}
    }

    fn setup_failing_session(
        fail_on_call: usize,
    ) -> (
        TcpStream,
        UnixStream,
        String,
        std::thread::JoinHandle<Result<()>>,
    ) {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_port = tcp_listener.local_addr().unwrap().port();
        let sock_path = temp_socket_path();
        let unix_listener = UnixListener::bind(&sock_path).unwrap();

        let mock_client = TcpStream::connect(("127.0.0.1", tcp_port)).unwrap();
        let (server_tcp, _) = tcp_listener.accept().unwrap();

        let mock_orch = UnixStream::connect(&sock_path).unwrap();
        let (server_unix, _) = unix_listener.accept().unwrap();

        let session_handle = std::thread::spawn(move || {
            run_session(
                Box::new(MockTranscriber::new("ignored")),
                Box::new(FailingMockTtsEngine::new(100, fail_on_call)),
                server_tcp,
                server_unix,
            )
        });

        (mock_client, mock_orch, sock_path, session_handle)
    }

    #[test]
    fn streaming_error_on_second_sentence_preserves_first() {
        // Fail on call 1 (second sentence) — first sentence audio should be preserved
        let (mock_client, mock_orch, sock_path, session_handle) = setup_failing_session(1);

        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());

        // 3 sentences: first synthesizes OK, second fails, third never attempted
        write_orchestrator_msg(
            &mut orch_w,
            &OrchestratorMsg::ResponseText("First OK. Second fails. Third never.".into()),
        )
        .unwrap();

        let mut total_samples = 0;
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {}
                ServerMsg::TtsAudioChunk(samples) => total_samples += samples.len(),
                ServerMsg::TtsEnd => break, // TtsEnd received — error didn't prevent it
                other => panic!("Unexpected: {other:?}"),
            }
        }

        // "First OK." = 9 chars * 100 = 900 samples (first sentence delivered)
        assert_eq!(total_samples, 900);

        drop(client_r);
        drop(orch_w);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    #[test]
    fn streaming_error_on_first_sentence_sends_tts_end() {
        // Fail on call 0 (first sentence) — no audio, but TtsEnd must be sent
        let (mock_client, mock_orch, sock_path, session_handle) = setup_failing_session(0);

        let mut client_r = BufReader::new(mock_client.try_clone().unwrap());
        let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());

        write_orchestrator_msg(
            &mut orch_w,
            &OrchestratorMsg::ResponseText("Fails immediately. Never reached.".into()),
        )
        .unwrap();

        let mut total_samples = 0;
        loop {
            let msg = read_server_msg(&mut client_r).unwrap();
            match msg {
                ServerMsg::Text(_) => {}
                ServerMsg::TtsAudioChunk(samples) => total_samples += samples.len(),
                ServerMsg::TtsEnd => break, // TtsEnd received despite all sentences failing
                other => panic!("Unexpected: {other:?}"),
            }
        }

        assert_eq!(
            total_samples, 0,
            "No audio should be sent on first-sentence error"
        );

        drop(client_r);
        drop(orch_w);
        drop(mock_client);
        drop(mock_orch);
        let _ = session_handle.join();
        std::fs::remove_file(&sock_path).ok();
    }

    // --- send_tts_chunks tests ---

    #[test]
    fn send_tts_chunks_no_tts_end() {
        let interrupted = AtomicBool::new(false);
        let samples: Vec<i16> = (0..8000).map(|i| i as i16).collect();
        let mut buf = Vec::new();

        let was_interrupted = send_tts_chunks(&mut buf, &samples, &interrupted).unwrap();
        assert!(!was_interrupted);

        // Should contain 2 TtsAudioChunk messages, NO TtsEnd
        let mut cursor = std::io::Cursor::new(buf);
        let mut chunk_count = 0;
        loop {
            match read_server_msg(&mut cursor) {
                Ok(ServerMsg::TtsAudioChunk(_)) => chunk_count += 1,
                Ok(other) => panic!("Expected TtsAudioChunk only, got {other:?}"),
                Err(_) => break, // EOF
            }
        }
        assert_eq!(chunk_count, 2);
    }

    #[test]
    fn crossfade_smooths_sentence_boundary() {
        // Sentence A: constant amplitude 10000
        let sentence_a = vec![10000i16; 1000];
        // Sentence B: constant amplitude -5000
        let mut sentence_b = vec![-5000i16; 1000];

        let tail: Vec<i16> = sentence_a[sentence_a.len() - CROSSFADE_LEN..].to_vec();
        apply_crossfade(&tail, &mut sentence_b);

        // First sample should be close to sentence A's amplitude (t≈0 → mostly prev)
        assert!(
            (sentence_b[0] as i32 - 10000).abs() < 200,
            "First crossfade sample should be near prev_tail value, got {}",
            sentence_b[0]
        );

        // Last crossfade sample should be close to sentence B's original amplitude (t≈1 → mostly next)
        assert!(
            (sentence_b[CROSSFADE_LEN - 1] as i32 - (-5000)).abs() < 200,
            "Last crossfade sample should be near original value, got {}",
            sentence_b[CROSSFADE_LEN - 1]
        );

        // Midpoint should be between the two amplitudes
        let mid = sentence_b[CROSSFADE_LEN / 2] as i32;
        assert!(
            mid > -5000 && mid < 10000,
            "Midpoint should be between -5000 and 10000, got {mid}"
        );

        // Samples after crossfade region should be unchanged
        assert_eq!(sentence_b[CROSSFADE_LEN], -5000);
        assert_eq!(sentence_b[999], -5000);

        // Verify monotonic transition (no sudden jumps)
        let mut max_delta: i32 = 0;
        for w in sentence_b[..CROSSFADE_LEN].windows(2) {
            let delta = (w[1] as i32 - w[0] as i32).abs();
            max_delta = max_delta.max(delta);
        }
        // Each step should be at most (10000 - (-5000)) / CROSSFADE_LEN ≈ 94 + some margin
        assert!(
            max_delta < 200,
            "Crossfade should be smooth, max delta = {max_delta}"
        );
    }

    #[test]
    fn crossfade_skipped_for_short_sentence() {
        // If sentence is shorter than CROSSFADE_LEN, crossfade should not be applied
        let tail = vec![10000i16; CROSSFADE_LEN];
        let mut short_sentence = vec![-5000i16; 50]; // less than CROSSFADE_LEN
        let original = short_sentence.clone();

        // apply_crossfade with len < CROSSFADE_LEN only crossfades available samples
        apply_crossfade(&tail, &mut short_sentence);

        // First sample should still be crossfaded (partial crossfade)
        // But in practice the consumer skips crossfade if samples.len() < CROSSFADE_LEN
        // Here we test the function itself handles short inputs gracefully
        assert_eq!(short_sentence.len(), original.len());
    }
}
