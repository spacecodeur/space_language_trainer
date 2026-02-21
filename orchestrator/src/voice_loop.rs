use anyhow::Result;
use std::io::{BufReader, BufWriter};
use std::os::unix::net::UnixStream;
use std::path::Path;

use space_lt_common::protocol::{
    OrchestratorMsg, ServerOrcMsg, is_disconnect, read_server_orc_msg, write_orchestrator_msg,
};
use space_lt_common::{info, warn};

use crate::claude::LlmBackend;

/// Truncate a string to at most `max_bytes` without splitting a UTF-8 character.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Find the last char boundary at or before max_bytes
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

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
        };

        // 2. Query LLM
        let prev_state = state;
        state = VoiceLoopState::QueryingLlm;
        info!("[orchestrator] State: {prev_state} → {state}");

        turn_count += 1;
        info!(
            "[orchestrator] Turn {turn_count}: received '{}'",
            truncate_utf8(&text, 80)
        );

        let response = match backend.query(&text, agent_path, turn_count > 1) {
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

        // 3. Send response back to server for TTS
        let prev_state = state;
        state = VoiceLoopState::WaitingForTts;
        info!("[orchestrator] State: {prev_state} → {state}");

        info!(
            "[orchestrator] Response: '{}'",
            truncate_utf8(&response, 80)
        );
        write_orchestrator_msg(writer, &OrchestratorMsg::ResponseText(response))?;

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
