mod claude;

use anyhow::Result;

use claude::{ClaudeCliBackend, LlmBackend, MockLlmBackend};
use space_lt_common::{debug, info};

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
            "Usage: space_lt_orchestrator --agent <path> [--session-dir <path>] [--mock] [--debug]"
        )
    })?;
    let agent_path = std::path::PathBuf::from(&agent_file);

    if !agent_path.exists() {
        anyhow::bail!("Agent file not found: {agent_file}");
    }

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

    info!("[orchestrator] Ready. Type text and press Enter (Ctrl+D to quit).");

    let stdin = std::io::stdin();
    let mut first_turn = true;

    loop {
        let mut line = String::new();
        let bytes_read = stdin.read_line(&mut line)?;
        if bytes_read == 0 {
            // EOF
            break;
        }

        let prompt = line.trim();
        if prompt.is_empty() {
            continue;
        }

        debug!(
            "[orchestrator] Sending prompt ({} chars, continue={})",
            prompt.len(),
            !first_turn
        );

        match backend.query(prompt, &agent_path, !first_turn) {
            Ok(response) => {
                println!("{response}");
                first_turn = false;
            }
            Err(e) => {
                eprintln!("[orchestrator] Error: {e}");
            }
        }
    }

    info!("[orchestrator] Session ended.");
    Ok(())
}
