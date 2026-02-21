mod claude;
mod connection;
mod voice_loop;

use anyhow::Result;
use std::net::Shutdown;

use claude::{ClaudeCliBackend, LlmBackend, MockLlmBackend};
use connection::OrchestratorConnection;
use space_lt_common::info;
use space_lt_common::protocol::{OrchestratorMsg, write_orchestrator_msg};

const DEFAULT_SOCKET_PATH: &str = "/tmp/space_lt_server.sock";

fn find_arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--debug") {
        space_lt_common::log::set_debug(true);
    }

    let agent_file = find_arg_value(&args, "--agent").ok_or_else(|| {
        anyhow::anyhow!(
            "Usage: space_lt_orchestrator --agent <path> [--socket <path>] [--session-dir <path>] [--mock] [--debug]"
        )
    })?;
    let agent_path = std::path::PathBuf::from(&agent_file);

    if !agent_path.exists() {
        anyhow::bail!("Agent file not found: {agent_file}");
    }

    let socket_path =
        find_arg_value(&args, "--socket").unwrap_or_else(|| DEFAULT_SOCKET_PATH.to_string());

    let session_dir = match find_arg_value(&args, "--session-dir") {
        Some(dir) => {
            let p = std::path::PathBuf::from(&dir);
            std::fs::create_dir_all(&p)?;
            p
        }
        None => {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let dir = std::env::temp_dir().join(format!("space_lt_orch_{timestamp}"));
            std::fs::create_dir_all(&dir)?;
            dir
        }
    };

    let use_mock = args.iter().any(|a| a == "--mock");

    // Build config JSON before session_dir is moved
    let config_json = format!(
        r#"{{"agent_file": "{}", "session_dir": "{}"}}"#,
        agent_path.display(),
        session_dir.display()
    );

    let backend: Box<dyn LlmBackend> = if use_mock {
        info!("[orchestrator] Using mock backend");
        Box::new(MockLlmBackend::new(vec![
            "Hello! I'm your English tutor. What would you like to practice today?".to_string(),
            "That's great! Let's keep going. Can you tell me more?".to_string(),
            "Excellent work! Your English is improving. Let's try another topic.".to_string(),
        ]))
    } else {
        info!("[orchestrator] Using Claude CLI backend");
        info!("[orchestrator] Session dir: {}", session_dir.display());
        Box::new(ClaudeCliBackend::new(session_dir))
    };

    // Connect to server via Unix socket
    let mut conn = OrchestratorConnection::connect(&socket_path)?;

    // Set up Ctrl+C handler: shutdown stream to unblock voice loop reader
    let shutdown_stream = conn.try_clone_stream()?;

    ctrlc::set_handler(move || {
        info!("[orchestrator] Ctrl+C received, shutting down...");
        let _ = shutdown_stream.shutdown(Shutdown::Both);
    })?;

    // Session start handshake
    conn.send_session_start(&config_json)?;

    // Run voice loop
    let (mut reader, mut writer) = conn.into_split();
    voice_loop::run_voice_loop(&mut reader, &mut writer, backend.as_ref(), &agent_path)?;

    // Attempt to send SessionEnd on exit (succeeds on normal exit; on Ctrl+C the
    // stream is already closed so this will fail — server detects disconnect instead)
    if let Err(e) = write_orchestrator_msg(&mut writer, &OrchestratorMsg::SessionEnd) {
        space_lt_common::debug!("[orchestrator] Could not send SessionEnd: {e}");
    }

    info!("[orchestrator] Session ended.");
    Ok(())
}
