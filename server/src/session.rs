use anyhow::{Context, Result};
use std::io::{BufReader, BufWriter, Write};
use std::net::{Shutdown, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

    // tcp_stream → writer for tts_router, tcp_for_read → reader for stt_router
    // unix_stream → writer for stt_router, unix_for_read → reader for tts_router

    // Shared pause state between stt_router and tts_router
    let paused = Arc::new(AtomicBool::new(false));
    let paused_stt = paused.clone();
    let paused_tts = paused;

    let stt_handle = std::thread::Builder::new()
        .name("stt_router".into())
        .spawn(move || stt_router(tcp_for_read, unix_stream, transcriber, paused_stt))?;

    let tts_handle = std::thread::Builder::new()
        .name("tts_router".into())
        .spawn(move || tts_router(unix_for_read, tcp_stream, tts, paused_tts))?;

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
        }
    }

    Ok(())
}

/// TTS routing: reads OrchestratorMsg from Unix, synthesizes speech, streams to client.
fn tts_router(
    unix_read: UnixStream,
    tcp_write: TcpStream,
    tts: Box<dyn TtsEngine>,
    paused: Arc<AtomicBool>,
) -> Result<()> {
    let mut reader = BufReader::new(unix_read);
    let mut writer = BufWriter::new(tcp_write);

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
                if paused.load(Ordering::SeqCst) {
                    debug!(
                        "[server] Paused — skipping TTS for response ({} chars)",
                        text.len()
                    );
                    write_server_msg(&mut writer, &ServerMsg::TtsEnd)?;
                    continue;
                }

                debug!("[server] ResponseText: {} chars", text.len());
                let tts_start = std::time::Instant::now();

                match tts.synthesize(&text) {
                    Ok(samples) => {
                        let synth_elapsed = tts_start.elapsed();
                        let audio_duration = samples.len() as f64 / 16000.0;
                        info!(
                            "[server] TTS synthesis: {:.2}s for {:.2}s of audio ({} chars)",
                            synth_elapsed.as_secs_f64(),
                            audio_duration,
                            text.len()
                        );
                        send_tts_audio(&mut writer, &samples)?;
                        info!(
                            "[server] TTS total (synthesis + send): {:.2}s",
                            tts_start.elapsed().as_secs_f64()
                        );
                    }
                    Err(e) => {
                        warn!("[server] TTS synthesis failed: {e}");
                        write_server_msg(&mut writer, &ServerMsg::TtsEnd)?;
                    }
                }
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
        }
    }

    Ok(())
}

/// Chunk TTS audio samples and send as TtsAudioChunk messages, followed by TtsEnd.
fn send_tts_audio(writer: &mut impl Write, samples: &[i16]) -> Result<()> {
    for chunk in samples.chunks(TTS_CHUNK_SIZE) {
        write_server_msg(writer, &ServerMsg::TtsAudioChunk(chunk.to_vec()))?;
    }
    write_server_msg(writer, &ServerMsg::TtsEnd)?;
    Ok(())
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
                ServerMsg::TtsAudioChunk(samples) => {
                    assert!(samples.len() <= TTS_CHUNK_SIZE);
                    total_samples.extend_from_slice(&samples);
                }
                ServerMsg::TtsEnd => break,
                other => panic!("Expected TtsAudioChunk or TtsEnd, got {other:?}"),
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
                other => panic!("Expected TtsAudioChunk or TtsEnd, got {other:?}"),
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

        // TTS during pause → only TtsEnd
        write_orchestrator_msg(&mut orch_w, &OrchestratorMsg::ResponseText("Skip".into())).unwrap();
        let msg = read_server_msg(&mut client_r).unwrap();
        match msg {
            ServerMsg::TtsEnd => {}
            other => panic!("Expected TtsEnd during pause, got {other:?}"),
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
                ServerMsg::TtsAudioChunk(_) => got_audio = true,
                ServerMsg::TtsEnd => break,
                other => panic!("Expected TtsAudioChunk or TtsEnd, got {other:?}"),
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
}
