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
    let server_addr = if server_addr.contains(':') {
        server_addr
    } else {
        format!("{server_addr}:9500")
    };

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

    // 3b. Replay support: shared buffer for last TTS response + clone of playback_tx
    let last_tts_audio: Arc<std::sync::Mutex<Vec<i16>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let last_tts_audio_writer = last_tts_audio.clone();
    let replay_tx = playback_tx.clone();

    // 4. Shutdown flag
    let shutdown = Arc::new(AtomicBool::new(false));

    // 5. TTS playback state tracking (for hotkey interrupt)
    let is_playing = Arc::new(AtomicBool::new(false));
    let is_playing_reader = is_playing.clone();

    // 5b. Cancel flags
    let cancel_pressed = Arc::new(AtomicBool::new(false));
    let cancel_requested = Arc::new(AtomicBool::new(false));
    let cancel_requested_reader = cancel_requested.clone();

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
                last_tts_audio_writer,
                cancel_requested_reader,
            )
        })?;

    // 7. Start audio capture
    let (audio_tx, audio_rx) = crossbeam_channel::bounded::<Vec<i16>>(64);
    let (_capture_stream, capture_config) = audio::start_capture(&config.device, audio_tx)?;
    let mut resample =
        audio::create_resampler(capture_config.sample_rate, 16000, capture_config.channels)?;

    // 8. Hotkey + cancel key
    let is_listening = Arc::new(AtomicBool::new(false));
    hotkey::listen_all_keyboards(
        config.hotkey,
        is_listening.clone(),
        Some(config.cancel_key),
        Some(cancel_pressed.clone()),
    )?;

    // 9. Ctrl+C handler
    let shutdown_clone = shutdown.clone();
    ctrlc::set_handler(move || {
        shutdown_clone.store(true, Ordering::SeqCst);
    })?;

    // 10. Main audio/VAD loop
    info!("Ready! Press {:?} to toggle listening.", config.hotkey);
    info!(
        "Press {:?} to cancel the current response.",
        config.cancel_key
    );

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

        // Check evdev cancel key (works during TTS and idle)
        if cancel_pressed
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            if is_playing.load(Ordering::SeqCst) {
                // During TTS: interrupt audio + cancel exchange
                info!("[CANCELLED]");
                let _ = write_client_msg(&mut writer, &ClientMsg::InterruptTts);
                let _ = write_client_msg(&mut writer, &ClientMsg::CancelExchange);
                is_playing.store(false, Ordering::SeqCst);
                playback_clear.store(true, Ordering::SeqCst);
                cancel_requested.store(true, Ordering::SeqCst);
                if let Ok(mut buf) = last_tts_audio.lock() {
                    buf.clear();
                }
            } else if !is_listening.load(Ordering::SeqCst) {
                // After TTS (idle) or during feedback: cancel exchange
                info!("[CANCELLED]");
                let _ = write_client_msg(&mut writer, &ClientMsg::CancelExchange);
                cancel_requested.store(true, Ordering::SeqCst);
                if let Ok(mut buf) = last_tts_audio.lock() {
                    buf.clear();
                }
            }
        }

        // Check for 'q' (quit), '3' (replay), or '4' (cancel) when not listening
        if !is_listening.load(Ordering::SeqCst) {
            match poll_key_action() {
                PollAction::Quit => {
                    info!("[client] Quit requested (q)");
                    quit_requested.store(true, Ordering::SeqCst);
                    break;
                }
                PollAction::Replay => {
                    if !is_playing.load(Ordering::SeqCst) {
                        replay_last_audio(&last_tts_audio, &replay_tx);
                    }
                }
                PollAction::Cancel => {
                    if !is_playing.load(Ordering::SeqCst) {
                        let has_audio = last_tts_audio
                            .lock()
                            .map(|b| !b.is_empty())
                            .unwrap_or(false);
                        if has_audio {
                            info!("[CANCELLED]");
                            let _ = write_client_msg(&mut writer, &ClientMsg::CancelExchange);
                            if let Ok(mut buf) = last_tts_audio.lock() {
                                buf.clear();
                            }
                        }
                    }
                }
                PollAction::None => {}
            }
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

/// Result of non-blocking key poll when not listening.
enum PollAction {
    None,
    Quit,
    Replay,
    Cancel,
}

/// Check for 'q' (quit), '3' (replay), or '4' (cancel) key press using crossterm polling (non-blocking).
fn poll_key_action() -> PollAction {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
    use crossterm::terminal;

    if terminal::is_raw_mode_enabled().unwrap_or(false) {
        return PollAction::None;
    }

    if terminal::enable_raw_mode().is_err() {
        return PollAction::None;
    }

    let action = if event::poll(Duration::from_millis(0)).unwrap_or(false) {
        match event::read() {
            Ok(Event::Key(KeyEvent {
                code: KeyCode::Char('q'),
                kind: KeyEventKind::Press,
                ..
            })) => PollAction::Quit,
            Ok(Event::Key(KeyEvent {
                code: KeyCode::Char('3'),
                kind: KeyEventKind::Press,
                ..
            })) => PollAction::Replay,
            Ok(Event::Key(KeyEvent {
                code: KeyCode::Char('4'),
                kind: KeyEventKind::Press,
                ..
            })) => PollAction::Cancel,
            _ => PollAction::None,
        }
    } else {
        PollAction::None
    };

    let _ = terminal::disable_raw_mode();
    action
}

/// Chunk size for replay playback (matches typical TTS chunk size).
const REPLAY_CHUNK_SIZE: usize = 4000;

/// Maximum replay buffer size in samples (~5 minutes at 16 kHz mono).
const REPLAY_BUFFER_MAX_SAMPLES: usize = 16_000 * 60 * 5;

/// Replay the last TTS response audio through the playback channel.
fn replay_last_audio(
    audio: &Arc<std::sync::Mutex<Vec<i16>>>,
    playback_tx: &crossbeam_channel::Sender<Vec<i16>>,
) {
    let samples = if let Ok(buf) = audio.lock() {
        buf.clone()
    } else {
        return;
    };
    if samples.is_empty() {
        return;
    }
    info!("[REPLAY]");
    for chunk in samples.chunks(REPLAY_CHUNK_SIZE) {
        if playback_tx.send(chunk.to_vec()).is_err() {
            break;
        }
    }
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

/// Feedback choice result.
enum FeedbackAction {
    Continue,
    Retry,
    Replay,
    Cancel,
}

/// Read a single keypress for feedback choice (no Enter needed).
/// Returns Continue ('1'), Retry ('2'), Replay ('3'), or Cancel ('4'/Esc).
/// Also checks `cancel_requested` for evdev cancel key pressed from main loop.
fn read_feedback_choice(
    shutdown: &Arc<AtomicBool>,
    cancel_requested: &Arc<AtomicBool>,
) -> FeedbackAction {
    use crossterm::event::{self, Event, KeyCode, KeyEvent};
    use crossterm::terminal;

    if terminal::enable_raw_mode().is_err() {
        // Fallback to line-based input if raw mode fails
        let mut input = String::new();
        let _ = std::io::stdin().read_line(&mut input);
        return match input.trim() {
            "2" => FeedbackAction::Retry,
            "3" => FeedbackAction::Replay,
            "4" => FeedbackAction::Cancel,
            _ => FeedbackAction::Continue,
        };
    }

    let result = loop {
        if shutdown.load(Ordering::SeqCst) {
            break FeedbackAction::Continue;
        }
        // Check if evdev cancel key was pressed (set by main loop)
        if cancel_requested.load(Ordering::SeqCst) {
            break FeedbackAction::Cancel;
        }
        // Poll with timeout so we can check shutdown + cancel_requested
        if event::poll(Duration::from_millis(200)).unwrap_or(false)
            && let Ok(Event::Key(KeyEvent { code, .. })) = event::read()
        {
            break match code {
                KeyCode::Char('1') => FeedbackAction::Continue,
                KeyCode::Char('2') => FeedbackAction::Retry,
                KeyCode::Char('3') => FeedbackAction::Replay,
                KeyCode::Char('4') | KeyCode::Esc => FeedbackAction::Cancel,
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
    for prefix in ["BLUE:", "YELLOW:"] {
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

/// Parse a corrected sentence, splitting on `<<` and `>>` delimiters.
///
/// Returns a vec of `(is_corrected, text)` segments.
/// - Text between `<<` and `>>` is marked as corrected (`true`).
/// - Text outside delimiters is unmarked (`false`).
/// - If no `<<` markers are found, the entire text is returned as `(true, text)` (graceful degradation).
/// - Empty input returns an empty vec.
/// - Unmatched `<<` (no closing `>>`) treats remaining text as corrected.
/// - Stray `>>` without preceding `<<` are treated as literal text.
fn parse_corrected_parts(text: &str) -> Vec<(bool, &str)> {
    if text.is_empty() {
        return Vec::new();
    }

    if !text.contains("<<") {
        return vec![(true, text)];
    }

    let mut parts = Vec::new();
    let mut remaining = text;

    while let Some(open) = remaining.find("<<") {
        // Text before `<<`
        let before = &remaining[..open];
        if !before.is_empty() {
            parts.push((false, before));
        }
        remaining = &remaining[open + 2..];

        // Find closing `>>`
        if let Some(close) = remaining.find(">>") {
            let inner = &remaining[..close];
            if !inner.is_empty() {
                parts.push((true, inner));
            }
            remaining = &remaining[close + 2..];
        } else {
            // Unmatched `<<` — treat rest as corrected
            if !remaining.is_empty() {
                parts.push((true, remaining));
            }
            remaining = "";
        }
    }

    // Any remaining text after the last `>>`
    if !remaining.is_empty() {
        parts.push((false, remaining));
    }

    parts
}

/// Display a corrected sentence line with green-highlighted corrected parts.
fn display_corrected_line(text: &str) {
    let trimmed = text.trim();
    let parts = parse_corrected_parts(trimmed);
    if parts.is_empty() {
        return;
    }
    eprint!("  \x1b[32m\u{2713}\x1b[0m ");
    for (is_corrected, segment) in &parts {
        if *is_corrected {
            eprint!("\x1b[32m{segment}\x1b[0m");
        } else {
            eprint!("{segment}");
        }
    }
    eprintln!("\x1b[0m");
}

/// Display language feedback with ANSI colors.
///
/// Lines prefixed with `RED:` are shown in red with a cross mark.
/// Lines prefixed with `BLUE:` are shown in blue with an arrow.
/// Other color prefixes Claude might invent are mapped to red or blue.
fn display_feedback(text: &str) {
    // Extract the LAST CORRECTED: line before the main loop
    // (must happen before classify_feedback_line to avoid ALLCAPS catch-all)
    let mut corrected_content: Option<&str> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.len() >= 10 && trimmed[..10].eq_ignore_ascii_case("CORRECTED:") {
            corrected_content = Some(trimmed[10..].trim());
        }
    }

    eprintln!("\x1b[2m--- feedback ---\x1b[0m");
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Skip CORRECTED lines (already extracted)
        if trimmed.len() >= 10 && trimmed[..10].eq_ignore_ascii_case("CORRECTED:") {
            continue;
        }
        let (severity, content) = classify_feedback_line(trimmed).unwrap_or(("blue", trimmed));
        match severity {
            "red" => eprintln!("  \x1b[31m\u{2717} {content}\x1b[0m"),
            _ => eprintln!("  \x1b[34m\u{279c} {content}\x1b[0m"),
        }
    }

    // Display corrected sentence last (green ✓)
    if let Some(content) = corrected_content {
        display_corrected_line(content);
    }

    eprintln!("\x1b[2m----------------\x1b[0m");
}

/// TCP reader loop: reads ServerMsg from TCP, routes TtsAudioChunk to playback.
#[allow(clippy::too_many_arguments)]
fn tcp_reader_loop(
    mut reader: BufReader<TcpStream>,
    mut feedback_writer: BufWriter<TcpStream>,
    playback_tx: crossbeam_channel::Sender<Vec<i16>>,
    output_rate: u32,
    shutdown: Arc<AtomicBool>,
    is_playing: Arc<AtomicBool>,
    summary_tx: crossbeam_channel::Sender<String>,
    last_tts_audio: Arc<std::sync::Mutex<Vec<i16>>>,
    cancel_requested: Arc<AtomicBool>,
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
                // Clear stale cancel from previous exchange on first chunk
                if !is_playing.load(Ordering::SeqCst) {
                    cancel_requested.store(false, Ordering::SeqCst);
                }
                is_playing.store(true, Ordering::SeqCst);
                let output = match &mut resample {
                    Some(r) => r(&samples),
                    None => samples,
                };
                // Accumulate for replay (capped to prevent unbounded growth)
                if let Ok(mut buf) = last_tts_audio.lock()
                    && buf.len() + output.len() <= REPLAY_BUFFER_MAX_SAMPLES
                {
                    buf.extend_from_slice(&output);
                }
                if playback_tx.send(output).is_err() {
                    debug!("[client] Playback channel closed");
                    break;
                }
            }
            ServerMsg::TtsEnd => {
                debug!("[client] TtsEnd received");
                // Flush resampler carry-over buffer (sends remaining samples)
                if let Some(r) = &mut resample {
                    let tail = r(&[]);
                    if !tail.is_empty() {
                        if let Ok(mut buf) = last_tts_audio.lock()
                            && buf.len() + tail.len() <= REPLAY_BUFFER_MAX_SAMPLES
                        {
                            buf.extend_from_slice(&tail);
                        }
                        let _ = playback_tx.send(tail);
                    }
                }
                is_playing.store(false, Ordering::SeqCst);
                // If cancel was triggered during TTS, skip feedback/replay
                if cancel_requested
                    .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    if let Ok(mut buf) = last_tts_audio.lock() {
                        buf.clear();
                    }
                    continue;
                }
                let has_audio = last_tts_audio
                    .lock()
                    .map(|buf| !buf.is_empty())
                    .unwrap_or(false);
                if has_audio {
                    eprintln!("  \x1b[2m[3] Replay  [4] Cancel\x1b[0m");
                }
            }
            ServerMsg::Ready => {
                debug!("[client] Unexpected Ready (ignoring)");
            }
            ServerMsg::Text(text) => {
                // Clear replay buffer and stale cancel when a new AI response starts
                if text.starts_with("AI:") {
                    cancel_requested.store(false, Ordering::SeqCst);
                    if let Ok(mut buf) = last_tts_audio.lock() {
                        buf.clear();
                    }
                }
                info!("[client] {text}");
            }
            ServerMsg::Error(err) => {
                warn!("[client] Server error: {err}");
            }
            ServerMsg::Feedback(text) => {
                display_feedback(&text);

                // Feedback choice loop (supports replay before deciding)
                // None = cancel, Some(true) = continue, Some(false) = retry
                let feedback_result: Option<bool> = loop {
                    eprintln!("  \x1b[1m[1] Continue  [2] Retry  [3] Replay  [4] Cancel\x1b[0m");
                    eprint!("  > ");
                    let _ = std::io::stderr().flush();

                    match read_feedback_choice(&shutdown, &cancel_requested) {
                        FeedbackAction::Replay => {
                            replay_last_audio(&last_tts_audio, &playback_tx);
                        }
                        FeedbackAction::Continue => break Some(true),
                        FeedbackAction::Retry => break Some(false),
                        FeedbackAction::Cancel => break None,
                    }
                };

                match feedback_result {
                    Some(proceed) => {
                        if let Err(e) = write_client_msg(
                            &mut feedback_writer,
                            &ClientMsg::FeedbackChoice(proceed),
                        ) {
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
                    None => {
                        // Cancel: check if evdev cancel already sent CancelExchange
                        let from_evdev = cancel_requested
                            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                            .is_ok();
                        if !from_evdev {
                            // Crossterm '4'/Esc — send CancelExchange ourselves
                            info!("[CANCELLED]");
                            if let Err(e) =
                                write_client_msg(&mut feedback_writer, &ClientMsg::CancelExchange)
                            {
                                if is_disconnect(&e) {
                                    debug!(
                                        "[client] Server disconnected while sending CancelExchange"
                                    );
                                    shutdown.store(true, Ordering::SeqCst);
                                    break;
                                }
                                warn!("[client] Failed to send CancelExchange: {e}");
                            }
                            if let Ok(mut buf) = last_tts_audio.lock() {
                                buf.clear();
                            }
                        }
                        // Don't send FeedbackChoice — orchestrator handles CancelExchange
                    }
                }
            }
            ServerMsg::StatusNotification(text) => {
                eprintln!("  \x1b[2;3m{text}\x1b[0m");
            }
            ServerMsg::SessionSummary(text) => {
                debug!("[client] SessionSummary: {} bytes", text.len());
                let _ = summary_tx.send(text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_corrected_parts_with_markers() {
        let result = parse_corrected_parts("I <<went>> to the store");
        assert_eq!(
            result,
            vec![(false, "I "), (true, "went"), (false, " to the store"),]
        );
    }

    #[test]
    fn parse_corrected_parts_without_markers() {
        let result = parse_corrected_parts("I went to the store");
        assert_eq!(result, vec![(true, "I went to the store")]);
    }

    #[test]
    fn parse_corrected_parts_multiple_markers() {
        let result = parse_corrected_parts("I <<went>> to <<the>> store");
        assert_eq!(
            result,
            vec![
                (false, "I "),
                (true, "went"),
                (false, " to "),
                (true, "the"),
                (false, " store"),
            ]
        );
    }

    #[test]
    fn parse_corrected_parts_empty() {
        let result = parse_corrected_parts("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_corrected_parts_unmatched_open() {
        let result = parse_corrected_parts("I <<went to the store");
        assert_eq!(result, vec![(false, "I "), (true, "went to the store")]);
    }

    #[test]
    fn parse_corrected_parts_adjacent_markers() {
        let result = parse_corrected_parts("<<foo>><<bar>>");
        assert_eq!(result, vec![(true, "foo"), (true, "bar")]);
    }

    #[test]
    fn parse_corrected_parts_stray_close() {
        // No `<<` at all, so `>>` is literal — graceful degradation: entire string is (true, ...)
        let result = parse_corrected_parts("I went>> to the store");
        assert_eq!(result, vec![(true, "I went>> to the store")]);
    }
}
