mod audio;
mod connection;
mod hotkey;
#[allow(dead_code)]
mod inject;
mod playback;
mod tui;
mod vad;

use anyhow::Result;
use space_lt_common::protocol::{ClientMsg, ServerMsg, write_client_msg};
use space_lt_common::{debug, info, warn};
use std::io::BufReader;
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use connection::is_disconnect;

fn find_arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn check_input_group() {
    // Check if current user is in the 'input' group (needed for evdev hotkey)
    let output = std::process::Command::new("id").arg("-Gn").output();
    match output {
        Ok(o) => {
            let groups = String::from_utf8_lossy(&o.stdout);
            if !groups.split_whitespace().any(|g| g == "input") {
                warn!("User is NOT in the 'input' group.");
                warn!("  This will block evdev hotkey access.");
                warn!("  Fix: sudo usermod -aG input $USER && log out/in");
            }
        }
        Err(_) => {
            warn!("Could not check group membership (id command failed).");
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--debug") {
        space_lt_common::log::set_debug(true);
    }

    let server_arg = find_arg_value(&args, "--server");
    run_client(server_arg)
}

fn run_client(server_override: Option<String>) -> Result<()> {
    info!("Space LT — Voice Conversation Client");
    check_input_group();

    // 1. TUI setup
    let config = tui::run_setup()?;

    let server_addr = server_override.unwrap_or(config.server_addr);

    info!("  Server:  {server_addr}");
    info!("  Device:  {}", config.device_name);
    info!("  Hotkey:  {:?}", config.hotkey);

    // 2. TCP connect + Ready handshake (with exponential backoff retry)
    info!("Connecting to server...");
    let conn = connection::TcpConnection::connect_with_retry(&server_addr)?;
    let shutdown_stream = conn.try_clone_stream()?;
    let (reader, writer) = conn.into_split();

    // 3. Start playback
    let (playback_tx, playback_rx) = crossbeam_channel::bounded::<Vec<i16>>(32);
    let (_playback_stream, output_rate) = playback::start_playback(playback_rx)?;

    // 4. Shutdown flag
    let shutdown = Arc::new(AtomicBool::new(false));

    // 5. Spawn tcp_reader thread
    let tcp_shutdown = shutdown.clone();
    let tcp_reader_handle = std::thread::Builder::new()
        .name("tcp_reader".into())
        .spawn(move || tcp_reader_loop(reader, playback_tx, output_rate, tcp_shutdown))?;

    // 6. Start audio capture
    let (audio_tx, audio_rx) = crossbeam_channel::bounded::<Vec<i16>>(64);
    let (_capture_stream, capture_config) = audio::start_capture(&config.device, audio_tx)?;
    let mut resample =
        audio::create_resampler(capture_config.sample_rate, 16000, capture_config.channels)?;

    // 7. Hotkey
    let is_listening = Arc::new(AtomicBool::new(false));
    hotkey::listen_all_keyboards(config.hotkey, is_listening.clone())?;

    // 8. Ctrl+C handler
    let shutdown_clone = shutdown.clone();
    ctrlc::set_handler(move || {
        shutdown_clone.store(true, Ordering::SeqCst);
    })?;

    // 9. Main audio/VAD loop
    info!("Ready! Press {:?} to toggle listening.", config.hotkey);

    let mut voice_detector = vad::VoiceDetector::new()?;
    let mut writer = writer;
    let mut was_listening = false;
    let mut chunk_count: u64 = 0;
    let mut listening_chunks: u64 = 0;

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        let chunk = match audio_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(c) => c,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        };

        chunk_count += 1;
        let listening = is_listening.load(Ordering::SeqCst);

        if was_listening && !listening {
            voice_detector.reset();
            if let Err(e) = write_client_msg(&mut writer, &ClientMsg::PauseRequest) {
                warn!("[client] Failed to send PauseRequest: {e}");
                if is_disconnect(&e) {
                    shutdown.store(true, Ordering::SeqCst);
                    break;
                }
            } else {
                info!("[client] Sent PauseRequest");
                info!("[PAUSED]");
            }
            debug!("  (processed {listening_chunks} audio chunks while listening)");
            listening_chunks = 0;
        }

        if !was_listening && listening {
            if let Err(e) = write_client_msg(&mut writer, &ClientMsg::ResumeRequest) {
                warn!("[client] Failed to send ResumeRequest: {e}");
                if is_disconnect(&e) {
                    shutdown.store(true, Ordering::SeqCst);
                    break;
                }
            } else {
                info!("[client] Sent ResumeRequest");
                info!("[LISTENING]");
            }
            listening_chunks = 0;
        }

        was_listening = listening;

        if !listening {
            if chunk_count.is_multiple_of(500) {
                debug!(
                    "  (audio flowing: {chunk_count} chunks received, {} samples/chunk)",
                    chunk.len()
                );
            }
            continue;
        }

        listening_chunks += 1;

        let resampled = resample(&chunk);
        if resampled.is_empty() {
            if listening_chunks.is_multiple_of(100) {
                debug!("  WARNING: resampler producing empty output");
            }
            continue;
        }

        if listening_chunks == 1 {
            debug!(
                "  Audio chunk: {} samples -> resampled to {} samples",
                chunk.len(),
                resampled.len()
            );
        }

        let segments = voice_detector.process_samples(&resampled);

        for segment in segments {
            let duration_ms = segment.len() as f64 / 16.0;
            debug!(
                "[SENDING...] segment: {} samples ({:.0}ms)",
                segment.len(),
                duration_ms
            );
            if let Err(e) = write_client_msg(&mut writer, &ClientMsg::AudioSegment(segment)) {
                if is_disconnect(&e) {
                    info!("[client] Server disconnected");
                    shutdown.store(true, Ordering::SeqCst);
                    break;
                }
                warn!("[client] Send error: {e}");
            }
        }
    }

    // 10. Graceful shutdown
    info!("Shutting down...");

    drop(_capture_stream);
    let _ = shutdown_stream.shutdown(Shutdown::Both);

    // Wait for tcp_reader thread with timeout
    let (done_tx, done_rx) = crossbeam_channel::bounded::<()>(1);
    std::thread::spawn(move || {
        let _ = tcp_reader_handle.join();
        let _ = done_tx.send(());
    });
    if done_rx.recv_timeout(Duration::from_secs(10)).is_err() {
        warn!("tcp_reader thread did not stop within 10s, exiting anyway.");
    }

    info!("Shutdown complete.");
    Ok(())
}

/// TCP reader loop: reads ServerMsg from TCP, routes TtsAudioChunk to playback.
fn tcp_reader_loop(
    mut reader: BufReader<TcpStream>,
    playback_tx: crossbeam_channel::Sender<Vec<i16>>,
    output_rate: u32,
    shutdown: Arc<AtomicBool>,
) {
    // Create resampler if playback device isn't 16kHz
    let mut resample: Option<audio::ResamplerFn> = if output_rate != 16000 {
        match audio::create_resampler(16000, output_rate, 1) {
            Ok(r) => {
                info!("[client] TTS resampling: 16kHz → {output_rate}Hz");
                Some(r)
            }
            Err(e) => {
                warn!("[client] Failed to create playback resampler: {e}");
                None
            }
        }
    } else {
        None
    };

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        let msg = match space_lt_common::protocol::read_server_msg(&mut reader) {
            Ok(msg) => msg,
            Err(e) => {
                if is_disconnect(&e) {
                    info!("[client] Server disconnected");
                } else {
                    warn!("[client] Read error: {e}");
                }
                shutdown.store(true, Ordering::SeqCst);
                break;
            }
        };

        match msg {
            ServerMsg::TtsAudioChunk(samples) => {
                debug!("[client] TtsAudioChunk: {} samples", samples.len());
                let output = match &mut resample {
                    Some(r) => r(&samples),
                    None => samples,
                };
                if playback_tx.send(output).is_err() {
                    debug!("[client] Playback channel closed");
                    break;
                }
            }
            ServerMsg::TtsEnd => {
                debug!("[client] TtsEnd received");
            }
            ServerMsg::Ready => {
                debug!("[client] Unexpected Ready (ignoring)");
            }
            ServerMsg::Text(text) => {
                debug!("[client] Unexpected Text: \"{text}\" (ignoring)");
            }
            ServerMsg::Error(err) => {
                warn!("[client] Server error: {err}");
            }
        }
    }
}
