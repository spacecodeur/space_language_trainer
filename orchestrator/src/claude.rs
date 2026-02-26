use anyhow::{Context, Result, bail};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Trait abstracting LLM invocation. Enables mock-based testing
/// without requiring real Claude CLI calls.
pub trait LlmBackend: Send + Sync {
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

    /// Query the LLM with a status notification channel.
    ///
    /// Backends that can detect intermediate states (e.g. web search in progress)
    /// send status strings on `status_tx`. Default: delegates to `query()`.
    fn query_with_status(
        &self,
        prompt: &str,
        system_prompt_file: &Path,
        continue_session: bool,
        _status_tx: std::sync::mpsc::Sender<String>,
    ) -> Result<String> {
        self.query(prompt, system_prompt_file, continue_session)
    }
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

/// Mock backend that fails a configurable number of times before returning responses.
/// Used for testing retry and error recovery logic.
#[cfg(test)]
pub struct FailingMockLlmBackend {
    fail_count: AtomicUsize,
    responses: Vec<String>,
    call_index: AtomicUsize,
}

#[cfg(test)]
impl FailingMockLlmBackend {
    pub fn new(fail_count: usize, responses: Vec<String>) -> Self {
        Self {
            fail_count: AtomicUsize::new(fail_count),
            responses,
            call_index: AtomicUsize::new(0),
        }
    }
}

#[cfg(test)]
impl LlmBackend for FailingMockLlmBackend {
    fn query(
        &self,
        _prompt: &str,
        _system_prompt_file: &Path,
        _continue_session: bool,
    ) -> Result<String> {
        let remaining = self.fail_count.load(Ordering::Relaxed);
        if remaining > 0 {
            self.fail_count.fetch_sub(1, Ordering::Relaxed);
            bail!("Simulated LLM failure");
        }
        if self.responses.is_empty() {
            bail!("FailingMockLlmBackend has no responses configured");
        }
        let i = self.call_index.fetch_add(1, Ordering::Relaxed);
        Ok(self.responses[i % self.responses.len()].clone())
    }
}

/// Real Claude CLI backend. Spawns `claude -p` per turn with timeout and retry.
pub struct ClaudeCliBackend {
    session_dir: std::path::PathBuf,
}

/// Maximum number of retry attempts for Claude CLI queries.
const MAX_RETRIES: u32 = 3;
/// Delay between retry attempts.
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(5);
/// Timeout for a single Claude CLI invocation.
const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Predefined error message sent to user via TTS when all retries fail.
const ERROR_FALLBACK: &str =
    "I'm sorry, I'm having trouble connecting right now. Please try again in a moment.";
/// Tools to enable for Claude CLI invocations.
/// WebSearch allows topic-based discussions with current information (FR12).
const ALLOWED_TOOLS: &str = "WebSearch";

impl ClaudeCliBackend {
    pub fn new(session_dir: std::path::PathBuf) -> Self {
        Self { session_dir }
    }

    /// Execute a single Claude CLI query with a 30-second timeout.
    ///
    /// If `status_tx` is provided, stderr is streamed line-by-line and web search
    /// activity is detected and reported via the channel.
    fn query_once(
        &self,
        prompt: &str,
        system_prompt_file: &Path,
        continue_session: bool,
        status_tx: Option<&std::sync::mpsc::Sender<String>>,
    ) -> Result<String> {
        use space_lt_common::debug;
        use std::io::{BufRead, Read, Write};
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

        cmd.args(["--output-format", "text", "--allowedTools", ALLOWED_TOOLS]);
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

        // Read stdout in background thread (blocks until process exits)
        let stdout_pipe = child.stdout.take().unwrap();
        let stdout_handle = std::thread::spawn(move || {
            let mut buf = String::new();
            std::io::BufReader::new(stdout_pipe)
                .read_to_string(&mut buf)
                .ok();
            buf
        });

        // Read stderr line-by-line to detect web search activity
        let stderr_pipe = child.stderr.take().unwrap();
        let status_tx_clone = status_tx.cloned();
        let stderr_handle = std::thread::spawn(move || {
            let mut collected = String::new();
            let reader = std::io::BufReader::new(stderr_pipe);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                debug!("[orchestrator] Claude stderr: {line}");
                // Detect web search activity from Claude CLI stderr
                if let Some(tx) = &status_tx_clone {
                    let lower = line.to_lowercase();
                    if lower.contains("web") && lower.contains("search") {
                        let _ = tx.send("Searching the web...".to_string());
                    }
                }
                collected.push_str(&line);
                collected.push('\n');
            }
            collected
        });

        // Poll child with try_wait() + timeout deadline
        let deadline = std::time::Instant::now() + QUERY_TIMEOUT;
        let status = loop {
            match child.try_wait().context("polling Claude CLI process")? {
                Some(status) => break status,
                None => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait(); // reap zombie
                        bail!("Claude CLI timed out after {}s", QUERY_TIMEOUT.as_secs());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        };

        // Collect output from reader threads
        let stdout_str = stdout_handle
            .join()
            .map_err(|_| anyhow::anyhow!("stdout reader thread panicked"))?;
        let stderr_str = stderr_handle
            .join()
            .map_err(|_| anyhow::anyhow!("stderr reader thread panicked"))?;

        if !status.success() {
            bail!("Claude CLI exited with {}: {}", status, stderr_str.trim());
        }

        let trimmed = stdout_str.trim().to_string();
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

impl LlmBackend for ClaudeCliBackend {
    fn query(
        &self,
        prompt: &str,
        system_prompt_file: &Path,
        continue_session: bool,
    ) -> Result<String> {
        self.query_with_retries(prompt, system_prompt_file, continue_session, None)
    }

    fn query_with_status(
        &self,
        prompt: &str,
        system_prompt_file: &Path,
        continue_session: bool,
        status_tx: std::sync::mpsc::Sender<String>,
    ) -> Result<String> {
        self.query_with_retries(
            prompt,
            system_prompt_file,
            continue_session,
            Some(&status_tx),
        )
    }
}

impl ClaudeCliBackend {
    fn query_with_retries(
        &self,
        prompt: &str,
        system_prompt_file: &Path,
        continue_session: bool,
        status_tx: Option<&std::sync::mpsc::Sender<String>>,
    ) -> Result<String> {
        use space_lt_common::{info, warn};

        // NOTE: if continue_session=true and a previous attempt was killed mid-response,
        // the Claude CLI session file may be in an inconsistent state. The retry with
        // --continue might fail for that reason. This is a known limitation.
        for attempt in 1..=MAX_RETRIES {
            match self.query_once(prompt, system_prompt_file, continue_session, status_tx) {
                Ok(response) => return Ok(response),
                Err(e) => {
                    warn!("[orchestrator] Claude CLI attempt {attempt}/{MAX_RETRIES} failed: {e}");
                    if attempt < MAX_RETRIES {
                        info!("[orchestrator] Retrying in {}s...", RETRY_DELAY.as_secs());
                        std::thread::sleep(RETRY_DELAY);
                    }
                }
            }
        }

        // All retries exhausted — return error fallback (NOT Err)
        warn!("[orchestrator] All {MAX_RETRIES} Claude CLI attempts failed, sending error to user");
        Ok(ERROR_FALLBACK.to_string())
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

    #[test]
    fn failing_mock_fails_then_succeeds() {
        let backend = FailingMockLlmBackend::new(2, vec!["Success".to_string()]);
        let p = PathBuf::from("agent.md");

        // First two calls fail
        assert!(backend.query("1", &p, false).is_err());
        assert!(backend.query("2", &p, true).is_err());

        // Third call succeeds
        let result = backend.query("3", &p, true).unwrap();
        assert_eq!(result, "Success");
    }

    #[test]
    fn failing_mock_immediate_success_when_zero_failures() {
        let backend = FailingMockLlmBackend::new(0, vec!["OK".to_string()]);
        let p = PathBuf::from("agent.md");

        let result = backend.query("1", &p, false).unwrap();
        assert_eq!(result, "OK");
    }

    #[test]
    fn allowed_tools_enables_web_search() {
        assert!(
            ALLOWED_TOOLS.contains("WebSearch"),
            "ALLOWED_TOOLS must include WebSearch for topic discussions (FR12)"
        );
        assert!(
            !ALLOWED_TOOLS.is_empty(),
            "ALLOWED_TOOLS must not be empty (empty string disables all tools)"
        );
    }
}
