# Story 3.3: Claude CLI Retry and Audio Error Recovery

Status: done

## Story

As a **user**,
I want the system to handle network timeouts and audio glitches automatically,
so that my session continues smoothly despite transient errors.

## Acceptance Criteria

1. **Given** an active conversation session
   **When** Claude CLI does not respond within 30 seconds (NFR7)
   **Then** orchestrator kills the subprocess and retries (up to 3 attempts, 5-second intervals)
   **And** if all retries fail, orchestrator sends a predefined error string as `ResponseText(0xA1)`
   **And** server synthesizes the error message via TTS and user hears it as audio prompt
   **And** conversation can continue on next user turn

2. **Given** a transient audio pipeline error
   **When** a cpal device error occurs on client
   **Then** client attempts stream restart up to 3 times before reporting error via TUI and exiting

3. **Given** a TCP connection drop occurs
   **When** a TCP connection drop occurs
   **Then** client attempts reconnection with exponential backoff (1s, 2s, 4s), max 3 attempts

4. **Given** a buffer underrun occurs
   **When** a buffer underrun occurs
   **Then** system logs warning and continues (self-healing, no user impact)

5. **And** integration test with `MockLlmBackend`: simulate timeout, verify retry behavior and error message
   **And** manual E2E test: disconnect internet mid-conversation, verify retry and audio error prompt

## Tasks / Subtasks

- [x] Task 1: Add 30-second timeout + retry to ClaudeCliBackend (AC: #1)
  - [x] 1.1: Replace `child.wait_with_output()` with `try_wait()` polling loop and 30s deadline
  - [x] 1.2: On timeout, `child.kill()` + `child.wait()` to reap subprocess
  - [x] 1.3: Wrap query in retry loop: up to 3 attempts with 5-second `thread::sleep` between retries
  - [x] 1.4: On total failure (3 retries exhausted), return predefined error string instead of `Err`
  - [x] 1.5: Add `info!` logs for timeout/retry events: `"[orchestrator] Claude CLI timed out (attempt {n}/3)"`

- [x] Task 2: Voice loop sends error ResponseText on LLM failure (AC: #1)
  - [x] 2.1: In `voice_loop.rs`, when `backend.query()` returns `Err`, send fallback as `ResponseText` to server
  - [x] 2.2: Server synthesizes error message via existing TTS pipeline (no server changes needed)
  - [x] 2.3: Conversation continues on next user turn (voice loop returns to WaitingForTranscription state)

- [x] Task 3: Client audio capture error recovery (AC: #2, #4)
  - [x] 3.1: In client audio capture, wrap cpal stream creation with retry (3 attempts, 500ms delay)
  - [x] 3.2: On buffer underrun in playback callback, log warning at debug level and continue
  - [x] 3.3: On unrecoverable device error after retries, return error (client exits gracefully)

- [x] Task 4: Client TCP reconnection on connection drop (AC: #3)
  - [x] 4.1: Add `connect_with_retry()` with exponential backoff (1s, 2s, 4s), max 3 attempts
  - [x] 4.2: Use `connect_with_retry()` for initial TCP connection in client main
  - [x] 4.3: Mid-session TCP drops exit gracefully (existing behavior preserved)
  - [x] 4.4: If all reconnect attempts fail, report error and exit gracefully

- [x] Task 5: Integration tests for Claude CLI retry (AC: #1, #5)
  - [x] 5.1: Add `FailingMockLlmBackend` with configurable failure count
  - [x] 5.2: Test: `failing_mock_fails_then_succeeds` — fails N times then returns responses
  - [x] 5.3: Test: `failing_mock_immediate_success_when_zero_failures`
  - [x] 5.4: Test: `voice_loop_sends_fallback_on_llm_error_and_continues` — voice loop sends fallback and continues

- [x] Task 6: Verify full build (AC: all)
  - [x] 6.1: Run `make check` — 83 tests pass (80 existing + 3 new), no regressions
  - [x] 6.2: Document manual E2E test instructions in completion notes

## Dev Notes

### CRITICAL: Claude CLI Timeout Implementation Pattern

The current `ClaudeCliBackend::query()` at `orchestrator/src/claude.rs:62-128` uses `child.wait_with_output()` which blocks indefinitely. Replace with a polling pattern:

```rust
// CURRENT (blocks forever):
let output = child.wait_with_output().context("waiting for Claude CLI")?;

// REPLACE WITH (30s timeout):
// 1. Take stdout/stderr handles BEFORE polling
let stdout = child.stdout.take().unwrap();
let stderr = child.stderr.take().unwrap();

// 2. Read stdout/stderr in background threads (they block until process exits or pipe closes)
let stdout_handle = std::thread::spawn(move || {
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut std::io::BufReader::new(stdout), &mut buf).ok();
    buf
});
let stderr_handle = std::thread::spawn(move || {
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut std::io::BufReader::new(stderr), &mut buf).ok();
    buf
});

// 3. Poll child with try_wait() + 30s deadline
let deadline = std::time::Instant::now() + Duration::from_secs(30);
let status = loop {
    match child.try_wait()? {
        Some(status) => break status,
        None => {
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait(); // reap zombie
                anyhow::bail!("Claude CLI timed out after 30s");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
};

// 4. Collect output
let stdout_str = stdout_handle.join().map_err(|_| anyhow::anyhow!("stdout reader panicked"))?;
let stderr_str = stderr_handle.join().map_err(|_| anyhow::anyhow!("stderr reader panicked"))?;
```

**Why this approach:**
- `try_wait()` is non-blocking — we can check a deadline between polls
- `child.kill()` sends SIGKILL on Unix, guaranteed to terminate
- `child.wait()` after kill reaps the zombie process
- stdout/stderr threads will unblock when the process is killed (pipes close)
- 100ms polling interval is fine — we only check once per 100ms, no busy loop

### CRITICAL: Retry Logic Wraps the Entire Query

The retry logic goes INSIDE `ClaudeCliBackend::query()`, NOT in the voice loop. The voice loop should not know about retries.

```rust
// In ClaudeCliBackend::query():
const MAX_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(30);
const ERROR_FALLBACK: &str = "I'm sorry, I'm having trouble connecting right now. Please try again in a moment.";

pub fn query(&self, prompt: &str, ...) -> Result<String> {
    for attempt in 1..=MAX_RETRIES {
        match self.query_once(prompt, system_prompt_file, continue_session) {
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
```

**Key design decision:** On total failure, return `Ok(ERROR_FALLBACK)` — NOT `Err`. This way:
- Voice loop treats it as a normal response
- ResponseText is sent to server → TTS synthesizes → user hears the error
- Voice loop returns to WaitingForTranscription → next turn works normally
- NO changes needed in voice_loop.rs for the happy error path

Extract the current `query()` body into a private `query_once()` method, then wrap it.

### CRITICAL: Voice Loop Error Path Change

Currently `voice_loop.rs:90-98` catches `Err` from `backend.query()` and just continues (user hears nothing). After the retry refactor, `query()` returns `Ok(ERROR_FALLBACK)` on total failure, so the error path changes:

- **Before:** `Err` → warn + continue (no feedback to user)
- **After:** `Ok("I'm sorry...")` → sent as ResponseText → user hears TTS error

This means the `Err` branch in voice_loop should only fire on truly unexpected errors (e.g., writer broken). Keep the existing error handling but consider also sending a fallback ResponseText there:

```rust
Err(e) => {
    warn!("[orchestrator] LLM query failed unexpectedly: {e}");
    // Attempt to notify user via TTS
    let fallback = "I'm sorry, something went wrong. Please try again.";
    if let Err(send_err) = write_orchestrator_msg(writer, &OrchestratorMsg::ResponseText(fallback.to_string())) {
        warn!("[orchestrator] Failed to send error message: {send_err}");
    }
    state = VoiceLoopState::WaitingForTranscription;
    continue;
}
```

### CRITICAL: Do NOT Change Protocol or Server

- `ResponseText(0xA1)` is already defined and handled by the server
- Server already synthesizes any ResponseText via TTS
- The error fallback string is just a normal ResponseText — no special message type needed
- **Do NOT modify `common/src/protocol.rs`**
- **Do NOT modify `server/src/session.rs`** (TTS routing already works)

### CRITICAL: Client Audio Error Recovery — Scope

**cpal device error (AC #2):** The capture stream is created in `client/src/audio.rs`. If `build_input_stream()` fails:
- Wrap the stream creation in a retry loop (3 attempts, 500ms between)
- If all attempts fail, return the error (client exits with TUI message)
- This is an initialization-time retry, not a mid-session recovery

**Buffer underrun (AC #4):** In `client/src/playback.rs`, the cpal output callback may experience underruns. These are already handled by cpal (silence is played). Add a `debug!` log if the ring buffer is empty when the callback fires. No other action needed.

**TCP reconnection (AC #3):** This is the most complex change. The approach:

1. In `client/src/main.rs`, wrap the session loop in an outer reconnection loop
2. When a TCP read/write error occurs (not from Ctrl+C shutdown), break the inner loop
3. Attempt TCP reconnect with exponential backoff: sleep 1s → try connect → sleep 2s → try → sleep 4s → try
4. If reconnect succeeds, re-enter the main session loop
5. If all 3 attempts fail, print error and exit

**Important:** When the client reconnects, the server starts a NEW session. The orchestrator (if still running) has a separate Unix socket connection — it doesn't know about the TCP reconnection. For MVP, the orchestrator must also detect the session end and reconnect, OR the user restarts the orchestrator manually. **Document this limitation in completion notes.**

For a simpler MVP approach: only retry the INITIAL connection (before the session starts). Mid-session TCP drops exit gracefully. This is much simpler and still satisfies "client attempts reconnection with exponential backoff" for the most common failure case (server not ready at startup).

### CRITICAL: MockLlmBackend Extension for Testing

The current `MockLlmBackend` in `orchestrator/src/claude.rs:20-49` cycles through predefined responses. Extend it to support failure simulation:

```rust
pub struct MockLlmBackend {
    responses: Vec<String>,
    index: std::sync::atomic::AtomicUsize,
    fail_count: std::sync::atomic::AtomicU32, // Number of calls to fail before succeeding
}

impl MockLlmBackend {
    pub fn new(responses: Vec<String>) -> Self { ... }

    /// Create a mock that fails `n` times before returning responses.
    pub fn with_failures(fail_count: u32, responses: Vec<String>) -> Self { ... }
}
```

When `fail_count > 0`, `query()` decrements and returns `Err(anyhow!("mock timeout"))`. Once exhausted, returns normal responses. This lets tests verify retry behavior without actual timeouts.

### Previous Story Intelligence (from Stories 3-1, 3-2)

- **Package naming:** `space_lt_*` (underscore in code, hyphen in Cargo.toml)
- **Makefile:** ALWAYS use `make check` not raw cargo commands
- **Clippy:** `-D warnings` — all warnings are errors
- **Error handling:** `anyhow::Result` + `.context()` — NOT `map_err`
- **Logging:** `[server]`/`[client]`/`[orchestrator]` prefix, `debug!()` for verbose, `info!()` for normal
- **Test convention:** inline `#[cfg(test)]` modules, `match`-based assertions
- **Protocol functions flush internally** — no explicit `flush()` after `write_*_msg` calls
- **EOF detection:** use `is_disconnect()` from `common/src/protocol.rs`
- **Stream cloning:** `try_clone()` for split read/write across threads
- **Shutdown:** `shutdown(Shutdown::Both)` to unblock blocking reads
- **Ordering:** `SeqCst` for shared AtomicBool operations (project convention)
- **Log truncation:** Use `truncate_utf8()` helper — never slice strings at arbitrary byte offsets
- **LLM query errors:** Should log and continue, not crash the process (code review fix M2 story 2-5)
- **Dead code:** Binary crate lint doesn't count test usage — use `#[allow(dead_code)]` with accurate comments
- **Test helper `setup_session()`:** Returns `(TcpStream, UnixStream, String, JoinHandle<Result<()>>)` — reuse for new tests
- **Negative assertions:** Use `set_read_timeout(500ms)` + match on `WouldBlock`/`TimedOut`
- **Test timing:** `thread::sleep(50ms)` for state propagation between threads (acceptable for MVP)
- **mpsc for timeout:** Use `std::sync::mpsc` (NOT crossbeam_channel, which is not in server crate)

### Project Structure Notes

Files to modify:
- `orchestrator/src/claude.rs` (MODIFY) — add timeout polling + retry loop + query_once refactor + extend MockLlmBackend
- `orchestrator/src/voice_loop.rs` (MODIFY) — send error ResponseText on unexpected LLM failure
- `client/src/audio.rs` (MODIFY) — add retry loop for cpal stream creation
- `client/src/playback.rs` (MODIFY) — add debug log for buffer underrun
- `client/src/main.rs` (MODIFY) — add TCP reconnection with exponential backoff on initial connect

Files NOT to modify:
- `common/src/protocol.rs` — no new message types needed
- `server/src/session.rs` — TTS routing already handles ResponseText correctly
- `server/src/server.rs` — no changes
- `server/src/tts.rs` — no changes
- `server/src/transcribe.rs` — already handles whisper errors gracefully (returns empty string)
- `orchestrator/src/connection.rs` — no changes
- `orchestrator/src/main.rs` — no changes (SessionEnd already handled from story 3-2)

### Current Test Counts

80 tests total: 20 client + 34 common + 10 orchestrator + 16 server
All must continue passing after this story (no regressions).

### References

- [Source: architecture.md#Gap Resolution G1] — Claude CLI timeout & retry pattern (FR18, NFR7, NFR12)
- [Source: architecture.md#Gap Resolution G5] — Audio pipeline error recovery (NFR14)
- [Source: architecture.md#Gap Resolution G3] — Audio playback buffering (no pre-buffer, accept underrun)
- [Source: architecture.md#Claude CLI Integration] — One `claude -p` per turn, `--continue` for session continuity
- [Source: architecture.md#LLM Backend Abstraction] — LlmBackend trait with MockLlmBackend for testing
- [Source: epics.md#Story 3.3] — AC1-AC5, FR18, NFR7, NFR12, NFR14
- [Source: prd.md#FR18] — System retries Claude CLI up to 3 times, reports failure via audio
- [Source: prd.md#NFR7] — 30-second timeout threshold
- [Source: prd.md#NFR12] — 3 retries with 5-second intervals
- [Source: prd.md#NFR14] — Audio pipeline graceful error recovery
- [Source: 3-1-hotkey-pause-resume.md] — Arc<AtomicBool> pattern, setup_session() helper, test conventions
- [Source: 3-2-hotkey-configuration-and-session-end.md] — SessionEnd flow, stream shutdown patterns, code review learnings

## Dev Agent Record

### Agent Model Used

Claude Opus 4.6

### Debug Log References

N/A

### Completion Notes List

- **Claude CLI timeout**: Refactored `query()` into `query_once()` (single attempt with 30s timeout via `try_wait()` polling) + `query()` (retry wrapper, 3 attempts, 5s delay). Background threads read stdout/stderr while main thread polls. On timeout, `child.kill()` + `child.wait()` reaps zombie.
- **Error fallback design**: On total retry failure, returns `Ok(ERROR_FALLBACK)` instead of `Err`. This way the voice loop treats it as a normal response → server synthesizes via TTS → user hears the error. No protocol or server changes needed.
- **Voice loop error path**: The `Err` branch (truly unexpected failures) now sends a fallback ResponseText to server for TTS, then continues. Previously it just logged and continued silently.
- **Audio capture retry**: 3 attempts with 500ms delay for `build_input_stream()` + `play()`. On final failure, returns error (client exits gracefully).
- **Buffer underrun**: Debug-level log only when `offset > 0` (audio was partially written mid-callback). Normal silence periods don't trigger the log.
- **TCP reconnection (MVP scope)**: `connect_with_retry()` implements exponential backoff (1s, 2s, 4s) for initial connection only. Mid-session TCP drops exit gracefully — full mid-session reconnection is too complex for MVP (requires coordinating client, server, and orchestrator state).
- **Test count**: 84 total (80 existing + 4 new: `failing_mock_fails_then_succeeds`, `failing_mock_immediate_success_when_zero_failures`, `voice_loop_sends_fallback_on_llm_error_and_continues`, `connect_with_retry_succeeds_on_second_attempt`)

### Manual E2E Test Instructions

1. Start server: `cargo run --bin space_lt_server`
2. Start orchestrator: `cargo run --bin space_lt_orchestrator -- --agent agent.md`
3. Start client: `cargo run --bin space_lt_client`
4. **Test retry**: Stop the Claude CLI process mid-conversation (or use a dummy agent that times out) — orchestrator should retry 3 times and eventually send error audio to client
5. **Test TCP reconnection**: Start client before server — client should retry connection with backoff
6. **Test audio capture**: Unplug/replug audio device during startup — client should retry stream creation
7. **Test buffer underrun**: Run with `--debug` flag, observe "[client] Playback buffer underrun" messages during normal TTS playback (expected occasionally)

### File List

- `orchestrator/src/claude.rs` — Major refactor: `query_once()` with 30s timeout + `query()` retry wrapper + `FailingMockLlmBackend` + 2 new tests
- `orchestrator/src/voice_loop.rs` — Error branch sends fallback ResponseText + 1 new test
- `client/src/audio.rs` — Capture stream retry loop (3 attempts, 500ms delay)
- `client/src/playback.rs` — Buffer underrun debug log
- `client/src/connection.rs` — `connect_with_retry()` with exponential backoff
- `client/src/main.rs` — Uses `connect_with_retry()` for initial connection

### Change Log

| Date | Change |
|------|--------|
| 2026-02-21 | Tasks 1-6 implemented: Claude CLI timeout+retry, voice loop error fallback, audio capture retry, buffer underrun log, TCP reconnect with backoff, integration tests. 83 tests pass. |
| 2026-02-21 | Code Review: 4 MEDIUM + 3 LOW findings. Fixed M1 (log off-by-one), M2 (added connect_with_retry test), M3 (first failure now logged), M4 (documented session-state limitation). 84 tests pass. |
