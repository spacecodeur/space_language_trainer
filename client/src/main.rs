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
use std::io::{BufReader, BufWriter, Write};
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

    debug!("  Server:  {server_addr}");
    debug!("  Device:  {}", config.device_name);
    debug!("  Hotkey:  {:?}", config.hotkey);
    debug!("  Mode:    {:?}", config.voice_mode);

    // 2. TCP connect + Ready handshake (with exponential backoff retry)
    debug!("Connecting to server...");
    let conn = connection::TcpConnection::connect_with_retry(&server_addr)?;
    let feedback_stream = conn.try_clone_stream()?;
    let shutdown_stream = conn.try_clone_stream()?;
    let (reader, writer) = conn.into_split();

    // 3. Start playback
    let (playback_tx, playback_rx) = crossbeam_channel::bounded::<Vec<i16>>(32);
    let playback_clear = Arc::new(AtomicBool::new(false));
    let (_playback_stream, output_rate) =
        playback::start_playback(playback_rx, playback_clear.clone())?;

    // 4. Shutdown flag
    let shutdown = Arc::new(AtomicBool::new(false));

    // 5. TTS playback state tracking (for hotkey interrupt)
    let is_playing = Arc::new(AtomicBool::new(false));
    let is_playing_reader = is_playing.clone();

    // 6. Spawn tcp_reader thread
    let tcp_shutdown = shutdown.clone();
    let (summary_tx, summary_rx) = crossbeam_channel::bounded::<String>(1);
    let tcp_reader_handle = std::thread::Builder::new()
        .name("tcp_reader".into())
        .spawn(move || {
            tcp_reader_loop(
                reader,
                BufWriter::new(feedback_stream),
                playback_tx,
                output_rate,
                tcp_shutdown,
                is_playing_reader,
                summary_tx,
            )
        })?;

    // 7. Start audio capture
    let (audio_tx, audio_rx) = crossbeam_channel::bounded::<Vec<i16>>(64);
    let (_capture_stream, capture_config) = audio::start_capture(&config.device, audio_tx)?;
    let mut resample =
        audio::create_resampler(capture_config.sample_rate, 16000, capture_config.channels)?;

    // 8. Hotkey
    let is_listening = Arc::new(AtomicBool::new(false));
    hotkey::listen_all_keyboards(config.hotkey, is_listening.clone())?;

    // 9. Ctrl+C handler
    let shutdown_clone = shutdown.clone();
    ctrlc::set_handler(move || {
        shutdown_clone.store(true, Ordering::SeqCst);
    })?;

    // 10. Main audio/VAD loop
    info!("Ready! Press {:?} to toggle listening.", config.hotkey);

    let voice_mode = config.voice_mode;
    let mut voice_detector = vad::VoiceDetector::new()?;
    let mut writer = writer;
    let mut was_listening = false;
    let mut chunk_count: u64 = 0;
    let mut listening_chunks: u64 = 0;
    let mut audio_accumulator: Vec<i16> = Vec::new(); // Manual mode: raw audio buffer
    let quit_requested = Arc::new(AtomicBool::new(false));

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        // Check for 'q' key (quit with optional summary)
        if !is_listening.load(Ordering::SeqCst) && poll_quit_key() {
            info!("[client] Quit requested (q)");
            quit_requested.store(true, Ordering::SeqCst);
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
            // Send accumulated audio before pausing
            match voice_mode {
                tui::VoiceMode::Manual => {
                    if !audio_accumulator.is_empty() {
                        let segment = std::mem::take(&mut audio_accumulator);
                        let duration_ms = segment.len() as f64 / 16.0;
                        debug!(
                            "[SENDING...] segment: {} samples ({:.0}ms)",
                            segment.len(),
                            duration_ms
                        );
                        if let Err(e) =
                            write_client_msg(&mut writer, &ClientMsg::AudioSegment(segment))
                        {
                            if is_disconnect(&e) {
                                shutdown.store(true, Ordering::SeqCst);
                                break;
                            }
                            warn!("[client] Send error: {e}");
                        }
                    }
                }
                tui::VoiceMode::Auto => {
                    // Flush any in-progress VAD segment before pausing
                    if let Some(segment) = voice_detector.flush() {
                        let duration_ms = segment.len() as f64 / 16.0;
                        debug!(
                            "[SENDING...] flush: {} samples ({:.0}ms)",
                            segment.len(),
                            duration_ms
                        );
                        if let Err(e) =
                            write_client_msg(&mut writer, &ClientMsg::AudioSegment(segment))
                        {
                            if is_disconnect(&e) {
                                shutdown.store(true, Ordering::SeqCst);
                                break;
                            }
                            warn!("[client] Send error: {e}");
                        }
                    }
                }
            }
            voice_detector.reset();
            if voice_mode == tui::VoiceMode::Auto {
                if let Err(e) = write_client_msg(&mut writer, &ClientMsg::PauseRequest) {
                    warn!("[client] Failed to send PauseRequest: {e}");
                    if is_disconnect(&e) {
                        shutdown.store(true, Ordering::SeqCst);
                        break;
                    }
                } else {
                    debug!("[client] Sent PauseRequest");
                }
            }
            info!("[PAUSED]");
            debug!("  (processed {listening_chunks} audio chunks while listening)");
            listening_chunks = 0;
        }

        if !was_listening && listening {
            // Interrupt TTS if currently playing (hotkey ON during playback)
            if is_playing.load(Ordering::SeqCst) {
                info!("[BARGE-IN] Hotkey interrupt");
                if let Err(e) = write_client_msg(&mut writer, &ClientMsg::InterruptTts) {
                    warn!("[client] Failed to send InterruptTts: {e}");
                    if is_disconnect(&e) {
                        shutdown.store(true, Ordering::SeqCst);
                        break;
                    }
                }
                is_playing.store(false, Ordering::SeqCst);
                playback_clear.store(true, Ordering::SeqCst);
            }
            audio_accumulator.clear();
            if voice_mode == tui::VoiceMode::Auto {
                if let Err(e) = write_client_msg(&mut writer, &ClientMsg::ResumeRequest) {
                    warn!("[client] Failed to send ResumeRequest: {e}");
                    if is_disconnect(&e) {
                        shutdown.store(true, Ordering::SeqCst);
                        break;
                    }
                } else {
                    debug!("[client] Sent ResumeRequest");
                }
            }
            info!("[LISTENING]");
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

        match voice_mode {
            tui::VoiceMode::Manual => {
                // Accumulate raw audio, send only on hotkey toggle-off
                audio_accumulator.extend_from_slice(&resampled);
            }
            tui::VoiceMode::Auto => {
                // VAD auto-segmentation: send segments when silence detected
                let segments = voice_detector.process_samples(&resampled);
                for segment in segments {
                    let duration_ms = segment.len() as f64 / 16.0;
                    debug!(
                        "[SENDING...] segment: {} samples ({:.0}ms)",
                        segment.len(),
                        duration_ms
                    );
                    if let Err(e) = write_client_msg(&mut writer, &ClientMsg::AudioSegment(segment))
                    {
                        if is_disconnect(&e) {
                            info!("[client] Server disconnected");
                            shutdown.store(true, Ordering::SeqCst);
                            break;
                        }
                        warn!("[client] Send error: {e}");
                    }
                }
            }
        }
    }

    // 11. Post-loop: summary prompt or direct shutdown
    drop(_capture_stream);

    if quit_requested.load(Ordering::SeqCst) && !shutdown.load(Ordering::SeqCst) {
        // User pressed 'q' — offer summary generation (TCP still open)
        eprintln!();
        eprintln!("  \x1b[1mGenerate session summary? [y/n]\x1b[0m");
        eprint!("  > ");
        let _ = std::io::stderr().flush();

        let generate = read_summary_choice(&shutdown);

        if generate {
            info!("Generating summary...");
            if let Err(e) = write_client_msg(&mut writer, &ClientMsg::SummaryRequest) {
                warn!("[client] Failed to send SummaryRequest: {e}");
            } else {
                match summary_rx.recv() {
                    Ok(summary) => match save_summary(&summary) {
                        Ok(path) => {
                            info!("Session summary saved to: {}", path.display());
                        }
                        Err(e) => {
                            warn!("[client] Failed to save summary: {e}");
                        }
                    },
                    Err(_) => {
                        warn!("[client] Summary channel closed before receiving response");
                    }
                }
            }
        }
    }

    // 12. Graceful shutdown
    info!("Shutting down...");
    shutdown.store(true, Ordering::SeqCst);
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

/// Check if 'q' key was pressed using crossterm polling (non-blocking).
fn poll_quit_key() -> bool {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
    use crossterm::terminal;

    if terminal::is_raw_mode_enabled().unwrap_or(false) {
        return false;
    }

    if terminal::enable_raw_mode().is_err() {
        return false;
    }

    let quit = if event::poll(Duration::from_millis(0)).unwrap_or(false) {
        matches!(
            event::read(),
            Ok(Event::Key(KeyEvent {
                code: KeyCode::Char('q'),
                kind: KeyEventKind::Press,
                ..
            }))
        )
    } else {
        false
    };

    let _ = terminal::disable_raw_mode();
    quit
}

/// Read a single keypress for summary choice (y/n).
fn read_summary_choice(shutdown: &Arc<AtomicBool>) -> bool {
    use crossterm::event::{self, Event, KeyCode, KeyEvent};
    use crossterm::terminal;

    if terminal::enable_raw_mode().is_err() {
        return false;
    }

    let result = loop {
        if shutdown.load(Ordering::SeqCst) {
            break false;
        }
        if event::poll(Duration::from_millis(500)).unwrap_or(false)
            && let Ok(Event::Key(KeyEvent { code, .. })) = event::read()
        {
            break match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => true,
                KeyCode::Char('n') | KeyCode::Char('N') => false,
                _ => continue,
            };
        }
    };

    let _ = terminal::disable_raw_mode();
    eprintln!();
    result
}

/// Save a session summary to ~/space-lt-sessions/YYYY-MM-DD_HH-MM.md
fn save_summary(content: &str) -> Result<std::path::PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let dir = std::path::PathBuf::from(home).join("space-lt-sessions");
    std::fs::create_dir_all(&dir)?;

    let filename = format!("{}.md", format_timestamp());
    let path = dir.join(filename);
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Format current local time as YYYY-MM-DD_HH-MM.
fn format_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
    )
}

/// Read a single keypress for feedback choice (no Enter needed).
/// Returns true for "continue" (any key except '2'), false for "retry" ('2').
fn read_single_key_choice(shutdown: &Arc<AtomicBool>) -> bool {
    use crossterm::event::{self, Event, KeyCode, KeyEvent};
    use crossterm::terminal;

    if terminal::enable_raw_mode().is_err() {
        // Fallback to line-based input if raw mode fails
        let mut input = String::new();
        let _ = std::io::stdin().read_line(&mut input);
        return input.trim() != "2";
    }

    let result = loop {
        if shutdown.load(Ordering::SeqCst) {
            break true;
        }
        // Poll with timeout so we can check shutdown
        if event::poll(Duration::from_millis(500)).unwrap_or(false)
            && let Ok(Event::Key(KeyEvent { code, .. })) = event::read()
        {
            break match code {
                KeyCode::Char('2') => false,
                KeyCode::Char('1') => true,
                _ => continue, // ignore other keys
            };
        }
    };

    let _ = terminal::disable_raw_mode();
    eprintln!(); // newline after the ">" prompt
    result
}

/// Strip a color-like prefix from a feedback line (e.g. "RED:", "YELLOW:", "GREEN:").
/// Returns the severity ("red" or "blue") and the remaining text.
fn classify_feedback_line(line: &str) -> Option<(&str, &str)> {
    // Known hard-error prefixes → red
    for prefix in ["RED:", "ORANGE:"] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(("red", rest.trim()));
        }
    }
    // Known soft-suggestion prefixes → blue
    for prefix in ["BLUE:", "YELLOW:", "GREEN:"] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(("blue", rest.trim()));
        }
    }
    // Any other ALLCAPS_WORD: prefix Claude might invent → blue, strip the prefix
    if let Some(colon_pos) = line.find(':') {
        let candidate = &line[..colon_pos];
        if !candidate.is_empty()
            && candidate
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_')
        {
            return Some(("blue", line[colon_pos + 1..].trim()));
        }
    }
    None
}

/// Display language feedback with ANSI colors.
///
/// Lines prefixed with `RED:` are shown in red with a cross mark.
/// Lines prefixed with `BLUE:` are shown in blue with an arrow.
/// Other color prefixes Claude might invent are mapped to red or blue.
fn display_feedback(text: &str) {
    eprintln!("\x1b[2m--- feedback ---\x1b[0m");
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (severity, content) = classify_feedback_line(trimmed).unwrap_or(("blue", trimmed));
        match severity {
            "red" => eprintln!("  \x1b[31m\u{2717} {content}\x1b[0m"),
            _ => eprintln!("  \x1b[34m\u{279c} {content}\x1b[0m"),
        }
    }
    eprintln!("\x1b[2m----------------\x1b[0m");
}

/// TCP reader loop: reads ServerMsg from TCP, routes TtsAudioChunk to playback.
fn tcp_reader_loop(
    mut reader: BufReader<TcpStream>,
    mut feedback_writer: BufWriter<TcpStream>,
    playback_tx: crossbeam_channel::Sender<Vec<i16>>,
    output_rate: u32,
    shutdown: Arc<AtomicBool>,
    is_playing: Arc<AtomicBool>,
    summary_tx: crossbeam_channel::Sender<String>,
) {
    // Create resampler if playback device isn't 16kHz
    let mut resample: Option<audio::ResamplerFn> = if output_rate != 16000 {
        match audio::create_resampler(16000, output_rate, 1) {
            Ok(r) => {
                debug!("[client] TTS resampling: 16kHz → {output_rate}Hz");
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
                    debug!("[client] Server disconnected");
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
                is_playing.store(true, Ordering::SeqCst);
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
                is_playing.store(false, Ordering::SeqCst);
            }
            ServerMsg::Ready => {
                debug!("[client] Unexpected Ready (ignoring)");
            }
            ServerMsg::Text(text) => {
                info!("[client] {text}");
            }
            ServerMsg::Error(err) => {
                warn!("[client] Server error: {err}");
            }
            ServerMsg::Feedback(text) => {
                display_feedback(&text);
                eprintln!("  \x1b[1m[1] Continue  [2] Retry and re-speak\x1b[0m");
                eprint!("  > ");
                let _ = std::io::stderr().flush();

                // Single-keypress read using crossterm raw mode
                let proceed = read_single_key_choice(&shutdown);

                if let Err(e) =
                    write_client_msg(&mut feedback_writer, &ClientMsg::FeedbackChoice(proceed))
                {
                    if is_disconnect(&e) {
                        debug!("[client] Server disconnected while sending FeedbackChoice");
                        shutdown.store(true, Ordering::SeqCst);
                        break;
                    }
                    warn!("[client] Failed to send FeedbackChoice: {e}");
                }

                if proceed {
                    info!("[client] Continuing with AI response...");
                } else {
                    info!("[client] Retrying — please re-speak your sentence.");
                }
            }
            ServerMsg::SessionSummary(text) => {
                debug!("[client] SessionSummary: {} bytes", text.len());
                let _ = summary_tx.send(text);
            }
        }
    }
}
