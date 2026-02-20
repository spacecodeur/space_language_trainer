use anyhow::{Context, Result, bail};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Trait abstracting LLM invocation. Enables mock-based testing
/// without requiring real Claude CLI calls.
pub trait LlmBackend: Send {
    /// Query the LLM with a prompt.
    ///
    /// - `system_prompt_file`: path to agent definition file (read on first turn)
    /// - `continue_session`: false for first turn (sends system prompt), true for subsequent turns
    fn query(
        &self,
        prompt: &str,
        system_prompt_file: &Path,
        continue_session: bool,
    ) -> Result<String>;
}

/// Mock backend returning predefined responses in order, cycling when exhausted.
pub struct MockLlmBackend {
    responses: Vec<String>,
    index: AtomicUsize,
}

impl MockLlmBackend {
    pub fn new(responses: Vec<String>) -> Self {
        Self {
            responses,
            index: AtomicUsize::new(0),
        }
    }
}

impl LlmBackend for MockLlmBackend {
    fn query(
        &self,
        _prompt: &str,
        _system_prompt_file: &Path,
        _continue_session: bool,
    ) -> Result<String> {
        if self.responses.is_empty() {
            bail!("MockLlmBackend has no responses configured");
        }
        let i = self.index.fetch_add(1, Ordering::Relaxed);
        Ok(self.responses[i % self.responses.len()].clone())
    }
}

/// Real Claude CLI backend. Spawns `claude -p` per turn.
pub struct ClaudeCliBackend {
    session_dir: std::path::PathBuf,
}

impl ClaudeCliBackend {
    pub fn new(session_dir: std::path::PathBuf) -> Self {
        Self { session_dir }
    }
}

impl LlmBackend for ClaudeCliBackend {
    fn query(
        &self,
        prompt: &str,
        system_prompt_file: &Path,
        continue_session: bool,
    ) -> Result<String> {
        use space_lt_common::debug;
        use std::io::Write;
        use std::process::{Command, Stdio};

        debug!(
            "[orchestrator] Spawning claude -p (continue={})",
            continue_session
        );

        let mut cmd = Command::new("claude");
        cmd.arg("-p");

        if continue_session {
            cmd.arg("--continue");
        } else {
            let system_prompt = std::fs::read_to_string(system_prompt_file)
                .context("reading system prompt file")?;
            cmd.args(["--system-prompt", &system_prompt]);
        }

        cmd.args(["--output-format", "text", "--tools", ""]);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.current_dir(&self.session_dir);
        cmd.env_remove("CLAUDECODE");

        let mut child = cmd.spawn().context("spawning Claude CLI")?;

        // Write prompt to stdin, then close it
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes())?;
            // stdin drops here, closing the pipe
        }

        let output = child.wait_with_output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "Claude CLI exited with {}: {}",
                output.status,
                stderr.trim()
            );
        }

        let response =
            String::from_utf8(output.stdout).context("Claude CLI output is not valid UTF-8")?;

        let trimmed = response.trim().to_string();
        if trimmed.is_empty() {
            bail!("Claude CLI returned empty response");
        }

        debug!(
            "[orchestrator] Claude response received ({} bytes)",
            trimmed.len()
        );

        Ok(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn mock_backend_returns_response() {
        let backend = MockLlmBackend::new(vec!["Hello!".to_string()]);
        let result = backend
            .query("Hi", &PathBuf::from("agent.md"), false)
            .unwrap();
        assert_eq!(result, "Hello!");
    }

    #[test]
    fn mock_backend_cycles_responses() {
        let backend = MockLlmBackend::new(vec!["A".to_string(), "B".to_string()]);
        let p = PathBuf::from("agent.md");

        assert_eq!(backend.query("1", &p, false).unwrap(), "A");
        assert_eq!(backend.query("2", &p, true).unwrap(), "B");
        assert_eq!(backend.query("3", &p, true).unwrap(), "A"); // cycles
        assert_eq!(backend.query("4", &p, true).unwrap(), "B");
    }

    #[test]
    fn mock_backend_empty_responses_errors() {
        let backend = MockLlmBackend::new(vec![]);
        let result = backend.query("Hi", &PathBuf::from("agent.md"), false);
        assert!(result.is_err());
    }
}
