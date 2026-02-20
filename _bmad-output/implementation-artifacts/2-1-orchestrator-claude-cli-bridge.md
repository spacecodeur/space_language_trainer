# Story 2.1: Orchestrator Claude CLI Bridge

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **developer**,
I want the orchestrator to communicate with Claude CLI programmatically via a `LlmBackend` trait,
so that I can validate the highest-risk component first and enable mock-based testing.

## Acceptance Criteria

1. **Given** the orchestrator crate with `orchestrator/src/claude.rs`
   **When** the developer defines the `LlmBackend` trait
   **Then** it has the signature: `fn query(&self, prompt: &str, system_prompt_file: &Path, continue_session: bool) -> Result<String>` and requires `Send`

2. **Given** `ClaudeCliBackend` implementing `LlmBackend`
   **When** `query()` is called with `continue_session=false`
   **Then** it spawns `claude -p --system-prompt <content> --output-format text --tools ""` with the prompt piped via stdin, CWD set to the session directory, and `CLAUDECODE` env var removed

3. **Given** `ClaudeCliBackend` implementing `LlmBackend`
   **When** `query()` is called with `continue_session=true`
   **Then** it spawns `claude -p --continue --output-format text --tools ""` with the prompt piped via stdin and CWD set to the same session directory

4. **Given** `MockLlmBackend` implementing `LlmBackend`
   **When** `query()` is called multiple times
   **Then** it returns predefined responses in order, cycling back to the first when exhausted

5. **Given** the agent directory `agent/` at workspace root
   **When** examining `agent/language_trainer.agent.md`
   **Then** it contains a minimal English tutor persona (2-3 sentences, plain prose, no JSON)

6. **Given** all unit tests using `MockLlmBackend`
   **When** running `make check`
   **Then** the mock query interface is verified and all tests pass (fmt, clippy, existing + new tests)

## Tasks / Subtasks

- [x] Task 1: Create LlmBackend trait and MockLlmBackend (AC: #1, #4, #6)
  - [x]1.1: Create `orchestrator/src/claude.rs` with `LlmBackend` trait (`query(&self, prompt, system_prompt_file, continue_session) -> Result<String>`, `Send` bound)
  - [x]1.2: Implement `MockLlmBackend` struct with `Vec<String>` responses and `AtomicUsize` index (cycling)
  - [x]1.3: Add `mod claude;` to `orchestrator/src/main.rs`
  - [x]1.4: Add `mock_backend_returns_response` test
  - [x]1.5: Add `mock_backend_cycles_responses` test

- [x] Task 2: Implement ClaudeCliBackend (AC: #2, #3)
  - [x]2.1: Implement `ClaudeCliBackend` struct with `session_dir: PathBuf` field
  - [x]2.2: Implement `query()` for first turn (`continue_session=false`): read system prompt file, spawn `claude -p` with `--system-prompt <content>`, pipe prompt via stdin
  - [x]2.3: Implement `query()` for subsequent turns (`continue_session=true`): spawn `claude -p --continue`, pipe prompt via stdin
  - [x]2.4: Set CWD to `session_dir` via `Command::current_dir()` for `--continue` isolation
  - [x]2.5: Remove `CLAUDECODE` env var via `Command::env_remove("CLAUDECODE")`
  - [x]2.6: Always pass `--output-format text` and `--tools ""`
  - [x]2.7: Handle errors: non-zero exit code, empty stdout, IO errors
  - [x] 2.8: ~Timeout deferred to story 3-3~ — uses blocking `wait_with_output()` for now

- [x] Task 3: Create language_trainer.agent.md (AC: #5)
  - [x]3.1: Create `agent/` directory at workspace root
  - [x]3.2: Create `agent/language_trainer.agent.md` — minimal English tutor persona (2-3 sentences)

- [x] Task 4: Update orchestrator main.rs for manual E2E testing (AC: #6)
  - [x]4.1: Add `find_arg_value` helper (same pattern as `server/src/main.rs`)
  - [x]4.2: Parse CLI args: `--agent <path>` (required), `--session-dir <path>` (default: temp dir), `--mock` (use MockLlmBackend), `--debug`
  - [x]4.3: Implement simple REPL loop: read line from stdin → query backend → print response to stdout
  - [x]4.4: First line uses `continue_session=false`, subsequent lines use `continue_session=true`

- [x] Task 5: Verify build passes (AC: #6)
  - [x]5.1: Run `make check` — fmt + clippy + all tests must pass
  - [x]5.2: Verify no regressions (48 existing tests still pass)

## Dev Notes

### CRITICAL: Spike Findings (Story 1-2, GO Decision)

The Phase 0 spike validated Claude CLI `--continue` over 22 turns with 100% context recall. **These findings are non-negotiable implementation constraints:**

#### 1. `--system-prompt` NOT `--system-prompt-file`

The architecture document is **WRONG** about `--system-prompt-file`. That flag does **NOT exist**.

**Correct Rust pattern:**
```rust
let system_prompt = std::fs::read_to_string(system_prompt_file)?;
Command::new("claude")
    .args(["-p", "--system-prompt", &system_prompt, "--output-format", "text", "--tools", ""])
    // ...
```

#### 2. Prompt via Stdin (NOT Positional Arg)

`--tools` is a **variadic flag** that consumes subsequent positional arguments. Always pipe the prompt through stdin:

```rust
let mut child = Command::new("claude")
    .args(["-p", "--output-format", "text", "--tools", ""])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .current_dir(&self.session_dir)
    .env_remove("CLAUDECODE")
    .spawn()?;

// Write prompt to stdin, then close it
child.stdin.take().unwrap().write_all(prompt.as_bytes())?;
let output = child.wait_with_output()?;
```

**WARNING:** With `std::process::Command`, args are NOT shell-parsed, so technically `--tools ""` is two separate args `["--tools", ""]`. However, stdin piping is still recommended for consistency with spike results and to avoid any future arg parsing issues.

#### 3. CWD Isolation for `--continue`

`--continue` resumes "the most recent conversation in the current working directory". The `ClaudeCliBackend` **MUST** use `Command::current_dir(&self.session_dir)` to isolate sessions.

#### 4. `CLAUDECODE` Env Var

When running Claude CLI from within a Claude Code session, the `CLAUDECODE` env var causes conflicts. Always remove it: `Command::env_remove("CLAUDECODE")`.

### LlmBackend Trait Design

```rust
use anyhow::Result;
use std::path::Path;

pub trait LlmBackend: Send {
    fn query(
        &self,
        prompt: &str,
        system_prompt_file: &Path,
        continue_session: bool,
    ) -> Result<String>;
}
```

**Why `&self` (not `&mut self`):** The backend is stateless per-call. Session state (first vs subsequent turn) is tracked by the caller via `continue_session`. CWD-based session isolation means no mutable state in the backend itself.

**Why `Send` bound:** The orchestrator's voice loop (story 2-5) will run the backend from a worker thread.

### MockLlmBackend Design

```rust
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct MockLlmBackend {
    responses: Vec<String>,
    index: AtomicUsize,
}
```

Uses `AtomicUsize` for the index so `query(&self, ...)` works without `&mut self`. Cycling: `index % responses.len()`.

### ClaudeCliBackend: No Retry (Deferred to Story 3-3)

Story 3-3 (`claude-cli-retry-and-audio-error-recovery`) handles retry logic (3 attempts, 5s intervals). For story 2-1, implement only:
- Basic 30-second timeout (kill child process if exceeded)
- Non-zero exit code → anyhow error
- Empty stdout → anyhow error

**Timeout pattern:**
```rust
use std::time::Duration;
// After spawning and writing stdin:
let output = child.wait_with_output()?;
// For timeout: use a thread with sleep + kill, OR accept blocking for now
// Simplest approach: just use wait_with_output() (blocking, no timeout)
// Add timeout in story 3-3 when retry logic is implemented
```

**Decision:** Skip timeout implementation in 2-1. Use blocking `wait_with_output()`. Story 3-3 adds timeout + retry. This keeps 2-1 focused on the core bridge.

### CLI Arg Parsing Pattern

Follow the exact pattern from `server/src/main.rs`:

```rust
fn find_arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
```

### File Structure

```
orchestrator/
├── Cargo.toml              # existing — no new dependencies needed
├── src/
│   ├── main.rs             # REPL entry point + arg parsing
│   └── claude.rs           # LlmBackend trait + ClaudeCliBackend + MockLlmBackend + tests

agent/
└── language_trainer.agent.md   # NEW directory at workspace root
```

### language_trainer.agent.md Content

Minimal 2-3 sentence persona. Example:

```markdown
You are a patient, encouraging English language tutor. Help the user practice conversational English by engaging in natural dialogue. When you notice grammar or vocabulary errors, gently correct them inline without breaking the conversation flow.
```

No JSON, no metadata, no structured format. Just plain prose that works as a `--system-prompt` value.

### Orchestrator main.rs REPL

Simple blocking loop for manual testing. NOT the real voice loop (story 2-5).

```
Usage: space_lt_orchestrator --agent <path> [--session-dir <path>] [--mock] [--debug]
```

- `--agent`: path to agent.md file (required)
- `--session-dir`: directory for Claude CLI session isolation (default: create temp dir)
- `--mock`: use MockLlmBackend instead of real ClaudeCliBackend
- `--debug`: enable debug logging

The REPL reads lines from stdin, calls `backend.query()`, prints the response. First line uses `continue_session=false`, subsequent lines use `continue_session=true`.

### No New Cargo Dependencies

`orchestrator/Cargo.toml` already has `anyhow` and `space_lt_common`. That's sufficient:
- `std::process::Command` for subprocess management
- `std::io` for stdin/stdout REPL
- `std::fs::read_to_string` for loading agent file
- `std::sync::atomic::AtomicUsize` for mock cycling

### Logging Convention

Follow architecture enforcement: component-prefixed logs.

```rust
use space_lt_common::{info, debug};

debug!("[orchestrator] Spawning claude -p (continue={continue_session})");
info!("[orchestrator] Claude response received ({} bytes)", response.len());
```

### Previous Story Intelligence (from Stories 1-1, 1-2, 1-3)

- Workspace: 4 crates, `make check` passes (48 tests)
- Package naming: `space_lt_*` (underscore)
- Makefile exists — always use `make check` not raw cargo commands
- `.gitignore` excludes `/target`, `/.claude/`, `/spike/results/`
- Clippy runs with `-D warnings` — all warnings are errors
- Existing test convention: inline `#[cfg(test)]` modules, `match`-based assertions (no `PartialEq` on enums)
- Protocol enums: `ClientMsg`, `ServerMsg`, `OrchestratorMsg` — all in `common/src/protocol.rs`
- Arg parsing: `find_arg_value()` helper, `args.iter().any()` for flags

### References

- [Source: architecture.md#Core Architectural Decisions] — LlmBackend trait, ClaudeCliBackend, orchestrator architecture
- [Source: architecture.md#Claude CLI Integration] — One `claude -p` per turn, `--continue` for session continuity
- [Source: architecture.md#Implementation Patterns] — Naming, testing, error handling, logging conventions
- [Source: architecture.md#Gap Resolutions G1] — Timeout/retry pattern (timeout in 2-1, retry deferred to 3-3)
- [Source: docs/spike-claude-cli.md] — Phase 0 spike results, invocation patterns, critical caveats
- [Source: epics.md#Story 2.1] — Acceptance criteria, epic context

## Dev Agent Record

### Agent Model Used
claude-opus-4-6

### Debug Log References
None

### Completion Notes List
- LlmBackend trait defined with Send bound and query(&self, prompt, system_prompt_file, continue_session) signature
- ClaudeCliBackend spawns `claude -p` per turn with correct flags: --system-prompt (first turn), --continue (subsequent), --output-format text, --tools ""
- Prompt piped via stdin (not positional arg) per spike findings
- CWD set to session_dir via Command::current_dir() for --continue isolation
- CLAUDECODE env var removed via Command::env_remove()
- MockLlmBackend with AtomicUsize cycling index — 3 unit tests (returns, cycles, empty errors)
- language_trainer.agent.md: minimal English tutor persona (1 sentence, plain prose)
- main.rs REPL: --agent (required), --session-dir (optional), --mock, --debug flags
- First turn uses continue_session=false, subsequent turns use continue_session=true
- No new Cargo dependencies (std::process::Command + anyhow)
- `make check` passes: 51 tests (17 client, 26 common, 3 orchestrator, 5 server), fmt clean, clippy clean

### File List
- `orchestrator/src/claude.rs` — NEW: LlmBackend trait + ClaudeCliBackend + MockLlmBackend + 3 tests
- `orchestrator/src/main.rs` — Replaced placeholder with REPL entry point + arg parsing
- `agent/language_trainer.agent.md` — NEW: minimal English tutor persona
