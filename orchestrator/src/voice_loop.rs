use anyhow::Result;
use std::io::{BufReader, BufWriter};
use std::os::unix::net::UnixStream;
use std::path::Path;

use space_lt_common::protocol::{
    OrchestratorMsg, ServerOrcMsg, is_disconnect, read_server_orc_msg, write_orchestrator_msg,
};
use space_lt_common::{info, warn};

use crate::claude::LlmBackend;

/// Short reminder prepended to every user prompt to reinforce voice output rules.
/// On --continue turns, Claude may "forget" the system prompt's formatting rules,
/// especially when using web search. This inline reminder keeps it on track.
const FORMAT_REMINDER: &str = "[CRITICAL: Your response is spoken aloud by TTS. Write ONLY plain conversational sentences. No markdown, no formatting, no lists, no URLs, no sources. 1-3 sentences max. If you notice grammar errors or unnatural phrasing in the user's message, prepend a [FEEDBACK]...[/FEEDBACK] block before your spoken response.]\n\n";

/// Voice loop state (for logging).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoiceLoopState {
    WaitingForTranscription,
    QueryingLlm,
    WaitingForTts,
}

impl std::fmt::Display for VoiceLoopState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaitingForTranscription => write!(f, "WaitingForTranscription"),
            Self::QueryingLlm => write!(f, "QueryingLlm"),
            Self::WaitingForTts => write!(f, "WaitingForTts"),
        }
    }
}

/// Parse an optional `[FEEDBACK]...[/FEEDBACK]` block from the beginning of a response.
///
/// Returns `(Some(feedback_content), remaining_text)` if a valid block is found,
/// or `(None, original_text)` for graceful degradation (missing tags, malformed, etc.).
pub fn parse_feedback(text: String) -> (Option<String>, String) {
    let trimmed = text.trim_start();
    let Some(after_open) = trimmed.strip_prefix("[FEEDBACK]") else {
        return (None, text);
    };
    let Some(end_idx) = after_open.find("[/FEEDBACK]") else {
        // Malformed: opening tag but no closing tag — treat entire text as spoken
        return (None, text);
    };
    let feedback = after_open[..end_idx].trim();
    if feedback.is_empty() {
        // Empty feedback block — skip it, pass the rest as spoken text
        let rest = after_open[end_idx + "[/FEEDBACK]".len()..].trim_start();
        return (None, rest.to_string());
    }
    let rest = after_open[end_idx + "[/FEEDBACK]".len()..].trim_start();
    (Some(feedback.to_string()), rest.to_string())
}

/// Run the main voice loop: read transcriptions, query LLM, send responses.
///
/// Blocks until the server disconnects or an unrecoverable error occurs.
pub fn run_voice_loop(
    reader: &mut BufReader<UnixStream>,
    writer: &mut BufWriter<UnixStream>,
    backend: &dyn LlmBackend,
    agent_path: &Path,
) -> Result<()> {
    let mut turn_count: u32 = 0;
    let mut state = VoiceLoopState::WaitingForTranscription;
    let mut retry_context: Option<String> = None;

    loop {
        // 1. Wait for transcribed text from server
        let msg = match read_server_orc_msg(reader) {
            Ok(msg) => msg,
            Err(e) if is_disconnect(&e) => {
                info!("[orchestrator] Server disconnected");
                break;
            }
            Err(e) => return Err(e),
        };

        let text = match msg {
            ServerOrcMsg::TranscribedText(t) => t,
            ServerOrcMsg::Error(e) => {
                warn!("[orchestrator] Server error: {e}");
                continue;
            }
            ServerOrcMsg::Ready => {
                info!("[orchestrator] Unexpected Ready during voice loop");
                continue;
            }
            ServerOrcMsg::FeedbackChoice(_) => {
                warn!("[orchestrator] Unexpected FeedbackChoice outside feedback flow");
                continue;
            }
        };

        // 2. Query LLM
        let prev_state = state;
        state = VoiceLoopState::QueryingLlm;
        info!("[orchestrator] State: {prev_state} → {state}");

        turn_count += 1;
        info!("[orchestrator] Turn {turn_count}: received '{text}'");

        // Prepend retry context if user chose to rephrase on previous turn
        let augmented_prompt = if let Some(ctx) = retry_context.take() {
            format!("{FORMAT_REMINDER}{ctx}{text}")
        } else {
            format!("{FORMAT_REMINDER}{text}")
        };
        let query_start = std::time::Instant::now();

        let response = match backend.query(&augmented_prompt, agent_path, turn_count > 1) {
            Ok(r) => r,
            Err(e) => {
                warn!("[orchestrator] LLM query failed unexpectedly: {e}");
                // Attempt to notify user via TTS
                let fallback = "I'm sorry, something went wrong. Please try again.";
                if let Err(send_err) = write_orchestrator_msg(
                    writer,
                    &OrchestratorMsg::ResponseText(fallback.to_string()),
                ) {
                    warn!("[orchestrator] Failed to send error message: {send_err}");
                }
                state = VoiceLoopState::WaitingForTranscription;
                info!("[orchestrator] State: QueryingLlm → {state}");
                continue;
            }
        };

        // 3. Parse feedback and send response
        let prev_state = state;
        state = VoiceLoopState::WaitingForTts;
        info!(
            "[orchestrator] LLM query: {:.2}s",
            query_start.elapsed().as_secs_f64()
        );
        info!("[orchestrator] State: {prev_state} → {state}");

        let (feedback, spoken) = parse_feedback(response);

        if let Some(fb) = feedback {
            info!("[orchestrator] Feedback detected, sending to client");
            write_orchestrator_msg(writer, &OrchestratorMsg::FeedbackText(fb))?;

            // Wait for user's choice: continue or retry (ignore stray messages)
            let feedback_choice = loop {
                let choice_msg = match read_server_orc_msg(reader) {
                    Ok(msg) => msg,
                    Err(e) if is_disconnect(&e) => {
                        info!(
                            "[orchestrator] Server disconnected while waiting for feedback choice"
                        );
                        break None;
                    }
                    Err(e) => return Err(e),
                };
                match choice_msg {
                    ServerOrcMsg::FeedbackChoice(proceed) => break Some(proceed),
                    other => {
                        warn!(
                            "[orchestrator] Ignoring unexpected message while waiting for FeedbackChoice: {other:?}"
                        );
                        continue;
                    }
                }
            };

            match feedback_choice {
                Some(true) => {
                    info!("[orchestrator] User chose to continue");
                    info!("[orchestrator] Response: '{spoken}'");
                    write_orchestrator_msg(writer, &OrchestratorMsg::ResponseText(spoken))?;
                }
                Some(false) => {
                    info!("[orchestrator] User chose to retry — skipping response");
                    retry_context = Some(
                        "[The user chose to rephrase their previous statement. Their new attempt follows:]\n\n"
                            .to_string(),
                    );
                    state = VoiceLoopState::WaitingForTranscription;
                    info!("[orchestrator] State: WaitingForTts → {state}");
                    continue;
                }
                None => {
                    // Server disconnected
                    break;
                }
            }
        } else {
            info!("[orchestrator] Response: '{spoken}'");
            write_orchestrator_msg(writer, &OrchestratorMsg::ResponseText(spoken))?;
        }

        // 4. Back to waiting
        let prev_state = state;
        state = VoiceLoopState::WaitingForTranscription;
        info!("[orchestrator] State: {prev_state} → {state}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::MockLlmBackend;
    use space_lt_common::protocol::{read_orchestrator_msg, write_orchestrator_msg};
    use std::path::PathBuf;

    // --- parse_feedback tests ---

    #[test]
    fn parse_feedback_with_valid_block() {
        let input = "[FEEDBACK]\nRED: \"I have went\" → \"I went\" (past simple)\nBLUE: \"it is good\" → \"it's appealing\" (more natural)\n[/FEEDBACK]\nThat sounds great! What else did you do?".to_string();
        let (fb, spoken) = parse_feedback(input);
        assert!(fb.is_some());
        let fb = fb.unwrap();
        assert!(fb.contains("RED:"));
        assert!(fb.contains("BLUE:"));
        assert_eq!(spoken, "That sounds great! What else did you do?");
    }

    #[test]
    fn parse_feedback_without_block() {
        let input = "That sounds great! What else did you do?".to_string();
        let (fb, spoken) = parse_feedback(input.clone());
        assert!(fb.is_none());
        assert_eq!(spoken, input);
    }

    #[test]
    fn parse_feedback_empty_block() {
        let input = "[FEEDBACK]\n[/FEEDBACK]\nThat sounds great!".to_string();
        let (fb, spoken) = parse_feedback(input);
        assert!(fb.is_none());
        assert_eq!(spoken, "That sounds great!");
    }

    #[test]
    fn parse_feedback_malformed_no_closing_tag() {
        let input = "[FEEDBACK]\nRED: something\nThat sounds great!".to_string();
        let (fb, spoken) = parse_feedback(input.clone());
        assert!(fb.is_none());
        assert_eq!(spoken, input);
    }

    #[test]
    fn parse_feedback_with_speed_tag_after() {
        let input = "[FEEDBACK]\nRED: \"I have went\" → \"I went\"\n[/FEEDBACK]\n[SPEED:0.8] That sounds great!".to_string();
        let (fb, spoken) = parse_feedback(input);
        assert!(fb.is_some());
        assert!(fb.unwrap().contains("RED:"));
        assert_eq!(spoken, "[SPEED:0.8] That sounds great!");
    }

    #[test]
    fn parse_feedback_empty_input() {
        let (fb, spoken) = parse_feedback(String::new());
        assert!(fb.is_none());
        assert_eq!(spoken, "");
    }

    #[test]
    fn parse_feedback_with_leading_whitespace() {
        let input = "  \n[FEEDBACK]\nRED: error\n[/FEEDBACK]\nResponse here.".to_string();
        let (fb, spoken) = parse_feedback(input);
        assert!(fb.is_some());
        assert_eq!(fb.unwrap(), "RED: error");
        assert_eq!(spoken, "Response here.");
    }

    #[test]
    fn voice_loop_processes_transcription_and_sends_response() {
        let (orch_stream, server_stream) = UnixStream::pair().unwrap();

        let server_handle = std::thread::spawn(move || {
            let mut reader = BufReader::new(server_stream.try_clone().unwrap());
            let mut writer = BufWriter::new(server_stream);

            // Server sends TranscribedText
            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("How are you?".into()),
            )
            .unwrap();

            // Server reads ResponseText
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::ResponseText(t) => assert_eq!(t, "Mock response A"),
                other => panic!("Expected ResponseText, got {other:?}"),
            }

            // Close to stop the loop
            drop(writer);
            drop(reader);
        });

        let backend = MockLlmBackend::new(vec!["Mock response A".to_string()]);
        let mut reader = BufReader::new(orch_stream.try_clone().unwrap());
        let mut writer = BufWriter::new(orch_stream);
        let agent_path = PathBuf::from("agent.md");

        let result = run_voice_loop(&mut reader, &mut writer, &backend, &agent_path);
        assert!(result.is_ok());

        server_handle.join().unwrap();
    }

    #[test]
    fn voice_loop_handles_server_error_and_continues() {
        let (orch_stream, server_stream) = UnixStream::pair().unwrap();

        let server_handle = std::thread::spawn(move || {
            let mut reader = BufReader::new(server_stream.try_clone().unwrap());
            let mut writer = BufWriter::new(server_stream);

            // Server sends Error first (should be skipped)
            use space_lt_common::protocol::{ServerMsg, write_server_msg};
            write_server_msg(&mut writer, &ServerMsg::Error("transient error".into())).unwrap();

            // Then sends TranscribedText
            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("Hello".into()),
            )
            .unwrap();

            // Read the response
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::ResponseText(t) => assert_eq!(t, "Reply"),
                other => panic!("Expected ResponseText, got {other:?}"),
            }

            // Close
            drop(writer);
            drop(reader);
        });

        let backend = MockLlmBackend::new(vec!["Reply".to_string()]);
        let mut reader = BufReader::new(orch_stream.try_clone().unwrap());
        let mut writer = BufWriter::new(orch_stream);
        let agent_path = PathBuf::from("agent.md");

        let result = run_voice_loop(&mut reader, &mut writer, &backend, &agent_path);
        assert!(result.is_ok());

        server_handle.join().unwrap();
    }

    #[test]
    fn voice_loop_multi_turn_maintains_continue_flag() {
        let (orch_stream, server_stream) = UnixStream::pair().unwrap();

        let server_handle = std::thread::spawn(move || {
            let mut reader = BufReader::new(server_stream.try_clone().unwrap());
            let mut writer = BufWriter::new(server_stream);

            // Turn 1
            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("Turn one".into()),
            )
            .unwrap();
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::ResponseText(t) => assert_eq!(t, "A"),
                other => panic!("Expected ResponseText A, got {other:?}"),
            }

            // Turn 2
            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("Turn two".into()),
            )
            .unwrap();
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::ResponseText(t) => assert_eq!(t, "B"),
                other => panic!("Expected ResponseText B, got {other:?}"),
            }

            drop(writer);
            drop(reader);
        });

        let backend = MockLlmBackend::new(vec!["A".to_string(), "B".to_string()]);
        let mut reader = BufReader::new(orch_stream.try_clone().unwrap());
        let mut writer = BufWriter::new(orch_stream);
        let agent_path = PathBuf::from("agent.md");

        let result = run_voice_loop(&mut reader, &mut writer, &backend, &agent_path);
        assert!(result.is_ok());

        server_handle.join().unwrap();
    }

    #[test]
    fn voice_loop_sends_fallback_on_llm_error_and_continues() {
        use crate::claude::FailingMockLlmBackend;

        let (orch_stream, server_stream) = UnixStream::pair().unwrap();

        let server_handle = std::thread::spawn(move || {
            let mut reader = BufReader::new(server_stream.try_clone().unwrap());
            let mut writer = BufWriter::new(server_stream);

            // Turn 1: will hit LLM error path
            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("First question".into()),
            )
            .unwrap();

            // Should receive fallback error message
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::ResponseText(t) => {
                    assert_eq!(t, "I'm sorry, something went wrong. Please try again.");
                }
                other => panic!("Expected error fallback ResponseText, got {other:?}"),
            }

            // Turn 2: LLM succeeds this time
            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("Second question".into()),
            )
            .unwrap();

            // Should receive normal response
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::ResponseText(t) => assert_eq!(t, "Normal response"),
                other => panic!("Expected 'Normal response', got {other:?}"),
            }

            // Close to stop the loop
            drop(writer);
            drop(reader);
        });

        // Backend fails once, then succeeds
        let backend = FailingMockLlmBackend::new(1, vec!["Normal response".to_string()]);
        let mut reader = BufReader::new(orch_stream.try_clone().unwrap());
        let mut writer = BufWriter::new(orch_stream);
        let agent_path = PathBuf::from("agent.md");

        let result = run_voice_loop(&mut reader, &mut writer, &backend, &agent_path);
        assert!(result.is_ok());

        server_handle.join().unwrap();
    }

    #[test]
    fn voice_loop_feedback_continue_sends_response() {
        let (orch_stream, server_stream) = UnixStream::pair().unwrap();

        let server_handle = std::thread::spawn(move || {
            let mut reader = BufReader::new(server_stream.try_clone().unwrap());
            let mut writer = BufWriter::new(server_stream);

            // Server sends TranscribedText
            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("I have went to store".into()),
            )
            .unwrap();

            // Server reads FeedbackText (orchestrator parsed it from LLM response)
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match &msg {
                OrchestratorMsg::FeedbackText(fb) => assert!(fb.contains("RED:")),
                other => panic!("Expected FeedbackText, got {other:?}"),
            }

            // Server sends FeedbackChoice(true) — user chose continue
            write_orchestrator_msg(&mut writer, &OrchestratorMsg::FeedbackChoice(true)).unwrap();

            // Server reads ResponseText (spoken part only)
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::ResponseText(t) => {
                    assert_eq!(t, "That sounds good! Tell me more.");
                }
                other => panic!("Expected ResponseText, got {other:?}"),
            }

            drop(writer);
            drop(reader);
        });

        let backend = MockLlmBackend::new(vec![
            "[FEEDBACK]\nRED: \"I have went\" → \"I went\" (past simple)\n[/FEEDBACK]\nThat sounds good! Tell me more.".to_string(),
        ]);
        let mut reader = BufReader::new(orch_stream.try_clone().unwrap());
        let mut writer = BufWriter::new(orch_stream);
        let agent_path = PathBuf::from("agent.md");

        let result = run_voice_loop(&mut reader, &mut writer, &backend, &agent_path);
        assert!(result.is_ok());

        server_handle.join().unwrap();
    }

    #[test]
    fn voice_loop_feedback_retry_skips_response_and_waits() {
        let (orch_stream, server_stream) = UnixStream::pair().unwrap();

        let server_handle = std::thread::spawn(move || {
            let mut reader = BufReader::new(server_stream.try_clone().unwrap());
            let mut writer = BufWriter::new(server_stream);

            // Turn 1: user speaks with error
            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("I have went to store".into()),
            )
            .unwrap();

            // Server reads FeedbackText
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            assert!(matches!(msg, OrchestratorMsg::FeedbackText(_)));

            // Server sends FeedbackChoice(false) — user chose retry
            write_orchestrator_msg(&mut writer, &OrchestratorMsg::FeedbackChoice(false)).unwrap();

            // No ResponseText should come — orchestrator waits for next transcription

            // Turn 2: user retries with corrected speech
            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("I went to the store".into()),
            )
            .unwrap();

            // Server reads ResponseText for turn 2 (no feedback this time)
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::ResponseText(t) => assert_eq!(t, "Great job! Much better."),
                other => panic!("Expected ResponseText, got {other:?}"),
            }

            drop(writer);
            drop(reader);
        });

        let backend = MockLlmBackend::new(vec![
            "[FEEDBACK]\nRED: \"I have went\" → \"I went\"\n[/FEEDBACK]\nNice try!".to_string(),
            "Great job! Much better.".to_string(),
        ]);
        let mut reader = BufReader::new(orch_stream.try_clone().unwrap());
        let mut writer = BufWriter::new(orch_stream);
        let agent_path = PathBuf::from("agent.md");

        let result = run_voice_loop(&mut reader, &mut writer, &backend, &agent_path);
        assert!(result.is_ok());

        server_handle.join().unwrap();
    }

    #[test]
    fn voice_loop_no_feedback_sends_response_directly() {
        // Verify no regression: when there's no feedback block, behavior is identical
        let (orch_stream, server_stream) = UnixStream::pair().unwrap();

        let server_handle = std::thread::spawn(move || {
            let mut reader = BufReader::new(server_stream.try_clone().unwrap());
            let mut writer = BufWriter::new(server_stream);

            write_orchestrator_msg(
                &mut writer,
                &OrchestratorMsg::TranscribedText("I went to the store yesterday".into()),
            )
            .unwrap();

            // Should get ResponseText directly (no FeedbackText)
            let msg = read_orchestrator_msg(&mut reader).unwrap();
            match msg {
                OrchestratorMsg::ResponseText(t) => assert_eq!(t, "That's great!"),
                other => panic!("Expected ResponseText, got {other:?}"),
            }

            drop(writer);
            drop(reader);
        });

        let backend = MockLlmBackend::new(vec!["That's great!".to_string()]);
        let mut reader = BufReader::new(orch_stream.try_clone().unwrap());
        let mut writer = BufWriter::new(orch_stream);
        let agent_path = PathBuf::from("agent.md");

        let result = run_voice_loop(&mut reader, &mut writer, &backend, &agent_path);
        assert!(result.is_ok());

        server_handle.join().unwrap();
    }

    /// Integration test: full orchestrator flow with SessionStart handshake + voice loop.
    #[test]
    fn full_orchestrator_session_with_handshake() {
        use crate::connection::OrchestratorConnection;
        use space_lt_common::protocol::{ServerMsg, write_server_msg};
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir();
        let sock_path = dir.join(format!("space_lt_integ_{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&sock_path);

        let listener = UnixListener::bind(&sock_path).unwrap();

        let sock_str = sock_path.to_str().unwrap().to_string();
        let orch_handle = std::thread::spawn(move || {
            // Orchestrator connects and does SessionStart
            let mut conn = OrchestratorConnection::connect(&sock_str).unwrap();
            conn.send_session_start(r#"{"agent_file": "agent.md"}"#)
                .unwrap();

            // Run voice loop
            let backend = MockLlmBackend::new(vec!["Response one".to_string()]);
            let (mut reader, mut writer) = conn.into_split();
            let agent_path = PathBuf::from("agent.md");
            run_voice_loop(&mut reader, &mut writer, &backend, &agent_path).unwrap();
        });

        // Server side
        let (server_stream, _) = listener.accept().unwrap();
        let mut server_reader = BufReader::new(server_stream.try_clone().unwrap());
        let mut server_writer = BufWriter::new(server_stream);

        // Read SessionStart
        let msg = read_orchestrator_msg(&mut server_reader).unwrap();
        match msg {
            OrchestratorMsg::SessionStart(json) => {
                assert!(json.contains("agent_file"));
            }
            other => panic!("Expected SessionStart, got {other:?}"),
        }

        // Send Ready
        write_server_msg(&mut server_writer, &ServerMsg::Ready).unwrap();

        // Send TranscribedText
        write_orchestrator_msg(
            &mut server_writer,
            &OrchestratorMsg::TranscribedText("Hello there".into()),
        )
        .unwrap();

        // Read ResponseText
        let msg = read_orchestrator_msg(&mut server_reader).unwrap();
        match msg {
            OrchestratorMsg::ResponseText(t) => assert_eq!(t, "Response one"),
            other => panic!("Expected ResponseText, got {other:?}"),
        }

        // Close connection to end the loop
        drop(server_writer);
        drop(server_reader);

        orch_handle.join().unwrap();
        std::fs::remove_file(&sock_path).ok();
    }
}
