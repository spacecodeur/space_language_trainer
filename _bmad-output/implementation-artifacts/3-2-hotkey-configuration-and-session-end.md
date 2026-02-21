# Story 3.2: Hotkey Configuration and Session End

Status: done

## Story

As a **user**,
I want to configure my preferred hotkey at startup and cleanly end a session,
so that I can choose a key that works with my tablet and exit gracefully.

## Acceptance Criteria

1. **Given** the client is launching
   **When** the TUI setup wizard runs at startup
   **Then** hotkey selection is available (extended from existing `space_tts` TUI)
   **And** server IP and port configuration is included in TUI

2. **Given** an active session
   **When** the user returns to the keyboard and quits (Ctrl+C or quit command)
   **Then** client sends a clean disconnect to server
   **And** orchestrator detects session end and performs cleanup
   **And** all processes shut down gracefully without hanging threads or leaked resources
   **And** manual E2E test: configure hotkey in TUI, start session, quit cleanly, verify no zombie processes

## Tasks / Subtasks

- [x] Task 1: Validate AC1 — TUI hotkey configuration already works (AC: #1)
  - [x] 1.1: Verify `client/src/tui.rs` offers hotkey selection (F2-F12, ScrollLock, Pause) and server address input
  - [x] 1.2: Run client binary, confirm TUI presents both screens (server addr + hotkey selection)
  - [x] 1.3: Document in completion notes that AC1 is fully satisfied by existing code

- [x] Task 2: Server — handle SessionEnd in tts_router (AC: #2)
  - [x] 2.1: In `tts_router` match arm for `OrchestratorMsg::SessionEnd`: change stub `debug!()` to `info!("[server] SessionEnd received, stopping session")` + `break;`
  - [x] 2.2: Verify that `break` from tts_router triggers `run_session()` cleanup path (already implemented: poll loop detects `tts_handle.is_finished()`, shuts down both streams, joins threads)
  - [x] 2.3: In `tts_router` match arm for `OrchestratorMsg::SessionStart`: update stub log from "not yet implemented" to "unexpected" since SessionStart should only arrive at `server.rs` level

- [x] Task 3: Orchestrator — always send SessionEnd on exit (AC: #2)
  - [x] 3.1: In `orchestrator/src/main.rs`: remove the `if !shutdown_flag.load()` guard so SessionEnd is sent on BOTH normal exit AND Ctrl+C
  - [x] 3.2: SessionEnd sent via `write_orchestrator_msg` on the split writer (conn consumed by `into_split()` so helper not usable directly)
  - [x] 3.3: `#[allow(dead_code)]` kept on `send_session_end()` — binary crate dead_code lint doesn't see test usage, added "Used in tests" comment
  - [x] 3.4: Handle send failure gracefully (log warning, don't crash — stream may already be closed on Ctrl+C)

- [x] Task 4: Integration tests for session end (AC: #2)
  - [x] 4.1: Test: orchestrator sends SessionEnd → tts_router exits → session ends cleanly (no hanging threads)
  - [x] 4.2: Test: client disconnects (TCP close) → stt_router exits → session ends cleanly
  - [x] 4.3: Both tests use mpsc channel with 5s timeout to verify session thread exits (panics on hang)

- [x] Task 5: Verify full build (AC: all)
  - [x] 5.1: Run `make check` — 80 tests pass (78 existing + 2 new), no regressions
  - [x] 5.2: Manual E2E test instructions documented in completion notes

## Dev Notes

### CRITICAL: AC1 Is Already Fully Implemented (Do NOT Rewrite)

**The TUI setup wizard is FULLY WORKING.** Both hotkey selection and server address input are already implemented and functional:

```
CLIENT TUI (working):
  tui.rs: Screen 1 → server address input (text_input_screen, default "127.0.0.1:9500")
  tui.rs: Screen 2 → Push-to-Talk Key selection (F2, F3, F4, F9, F10, F11, F12, ScrollLock, Pause)
  Returns: SetupConfig { server_addr, device, device_name, hotkey }
```

The architecture doc explicitly states: **"No persistent config file for MVP — TUI-based setup at each launch"**. Do NOT add config file persistence — this is by design.

**Task 1 is a VALIDATION task only.** Run the client, confirm the TUI works, document it. Zero code changes.

### CRITICAL: What Needs Implementation (Session End)

The session end flow has 3 gaps to fill:

```
GAP 1 — Server tts_router ignores SessionEnd:
  session.rs line ~214: OrchestratorMsg::SessionEnd => { debug!("not yet implemented"); }
  FIX: Replace with info! + break;

GAP 2 — Orchestrator skips SessionEnd on Ctrl+C:
  orchestrator/src/main.rs lines ~104-108:
    if !shutdown_flag.load(Ordering::Relaxed) {
        let _ = write_orchestrator_msg(&mut writer, &OrchestratorMsg::SessionEnd);
    }
  FIX: Always send SessionEnd (remove the guard condition)

GAP 3 — send_session_end() helper is dead code:
  orchestrator/src/connection.rs: #[allow(dead_code)] pub fn send_session_end()
  FIX: Use it and remove dead_code attribute
```

### CRITICAL: Session End Flow After Fixes

**Orchestrator exits (normal or Ctrl+C):**
1. Orchestrator sends `SessionEnd(0xA3)` over Unix socket
2. `tts_router` receives it → logs info → `break;`
3. `run_session()` poll loop detects `tts_handle.is_finished()` → shuts down TCP + Unix streams
4. `stt_router` gets disconnect from shutdown TCP → `break;`
5. Both threads joined → `run_session()` returns → `run_daemon()` cleans up socket file
6. Server is ready for next session (or exits)

**Client exits (Ctrl+C):**
1. Client's `ctrlc` handler sets `shutdown` flag
2. Main loop breaks → drops `_capture_stream` → `shutdown_stream.shutdown(Shutdown::Both)`
3. TCP connection closes
4. `stt_router` on server gets client disconnect → `break;`
5. `run_session()` poll loop detects `stt_handle.is_finished()` → shuts down Unix stream
6. `tts_router` gets disconnect from shutdown Unix → `break;`
7. Both threads joined → session ends cleanly

**No changes needed to client shutdown logic** — it already works correctly by closing the TCP connection, which the server's `stt_router` detects as a disconnect.

### CRITICAL: No Changes to Protocol

`SessionEnd(0xA3)` is already defined in `common/src/protocol.rs` with:
- Encode: `write_orchestrator_msg` handles `SessionEnd` variant (empty payload)
- Decode: `read_orchestrator_msg` handles `SessionEnd` variant
- Tests: `round_trip_session_end` passes

**Do NOT modify `common/src/protocol.rs`.**

### CRITICAL: Orchestrator SessionEnd — Use Connection Helper

The orchestrator already has a `send_session_end()` method in `connection.rs`:

```rust
// orchestrator/src/connection.rs
#[allow(dead_code)]
pub fn send_session_end(&mut self) -> Result<()> {
    write_orchestrator_msg(&mut self.writer, &OrchestratorMsg::SessionEnd)
}
```

Currently unused (dead code). Story 3-2 should:
1. Use `conn.send_session_end()` instead of inline `write_orchestrator_msg` in `main.rs`
2. Remove the `#[allow(dead_code)]` attribute
3. Send SessionEnd unconditionally on exit (both normal and Ctrl+C paths)

The current inline code in `main.rs` (lines ~104-108) guards SessionEnd with `!shutdown_flag`, which means Ctrl+C skips it. Fix: move SessionEnd sending BEFORE the voice loop exit check, or remove the guard.

**Important:** On Ctrl+C, the Unix stream is already `shutdown(Shutdown::Both)` by the Ctrl+C handler. Sending SessionEnd after that will fail. The fix is to either:
- Option A: Send SessionEnd in the Ctrl+C handler BEFORE shutting down the stream
- Option B: Always attempt to send SessionEnd after voice loop exits, ignoring send failures

Option B is simpler and more robust: `let _ = conn.send_session_end();` (ignore errors).

### CRITICAL: SessionStart Log Update in tts_router

Story 3-1's code review (finding M1) explicitly reverted the SessionStart log change as "out of scope". Story 3-2 should now make this change properly:

```rust
// CURRENT (stub):
OrchestratorMsg::SessionStart(json) => {
    debug!("[server] SessionStart received (not yet implemented): {}", json);
}

// CHANGE TO:
OrchestratorMsg::SessionStart(json) => {
    debug!("[server] SessionStart in tts_router (unexpected): {}", json);
}
```

This is correct because `SessionStart` should be handled at the `server.rs` level (in `run_daemon()`), not inside `tts_router`. If it arrives in `tts_router`, it's unexpected.

### Test Strategy

**Integration tests in `server/src/session.rs`** (extend existing test module):

Reuse the `setup_session()` helper from story 3-1 tests. New tests focus on session lifecycle:

```rust
#[test]
fn session_end_stops_session() {
    // Setup: TCP + Unix connections + run_session thread
    let (mock_client, mock_orch, sock_path, session_handle) = setup_session("test", 4000);

    // Orchestrator sends SessionEnd
    let mut orch_w = BufWriter::new(mock_orch.try_clone().unwrap());
    write_orchestrator_msg(&mut orch_w, &OrchestratorMsg::SessionEnd).unwrap();

    // Session should end within a reasonable time
    let result = session_handle.join().expect("session thread should not panic");
    assert!(result.is_ok(), "session should end cleanly");

    // Cleanup
    drop(orch_w);
    drop(mock_client);
    drop(mock_orch);
    std::fs::remove_file(&sock_path).ok();
}

#[test]
fn client_disconnect_ends_session() {
    let (mock_client, mock_orch, sock_path, session_handle) = setup_session("test", 4000);

    // Client disconnects (close TCP)
    drop(mock_client);

    // Session should end cleanly
    let result = session_handle.join().expect("session thread should not panic");
    assert!(result.is_ok(), "session should end cleanly on client disconnect");

    // Cleanup
    drop(mock_orch);
    std::fs::remove_file(&sock_path).ok();
}
```

**Important test detail:** The `session_handle.join()` will block until the session ends. Use the fact that `setup_session()` returns the `JoinHandle<Result<()>>` — if the session hangs, the test will timeout (Rust test default: 60s, or `make check` timeout).

### Previous Story Intelligence (from Stories 2-5, 3-1)

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
- **Code review fix M1 (story 3-1):** SessionEnd/SessionStart log changes were reverted as out-of-scope — story 3-2 should now apply them properly
- **Code review fix M2 (story 3-1):** [PAUSED]/[LISTENING] logs moved to success branch — follow same pattern for any new user-visible state logs
- **Code review fix H1 (story 2-5):** Use `truncate_utf8()` helper for log truncation — never slice strings at arbitrary byte offsets
- **Code review fix M2 (story 2-5):** LLM query errors should log and continue, not crash the process
- **Test helper `setup_session()`:** Returns `(TcpStream, UnixStream, String, JoinHandle<Result<()>>)` — reuse for new tests
- **Negative assertions:** Use `set_read_timeout(500ms)` + match on `WouldBlock`/`TimedOut`
- **Test timing:** `thread::sleep(50ms)` for state propagation between threads (acceptable for MVP)

### Project Structure Notes

Files to modify:
- `server/src/session.rs` (MODIFY) — SessionEnd: break tts_router; SessionStart: update log; add 2 integration tests
- `orchestrator/src/main.rs` (MODIFY) — always send SessionEnd on exit (remove guard)
- `orchestrator/src/connection.rs` (MODIFY) — remove `#[allow(dead_code)]` from `send_session_end()`

Files NOT to modify:
- `client/src/main.rs` — shutdown logic already works (TCP close signals server)
- `client/src/tui.rs` — hotkey configuration already fully working
- `client/src/hotkey.rs` — working as-is
- `client/src/connection.rs` — no changes
- `common/src/protocol.rs` — SessionEnd already defined
- `server/src/server.rs` — no changes (SessionStart handshake already there)
- `server/src/listener.rs` — no changes

### Current Test Counts

78 tests total: 20 client + 34 common + 10 orchestrator + 14 server
All must continue passing after this story (no regressions).

### References

- [Source: architecture.md#Gap Resolutions G6] — Session start handshake (5-step sequence)
- [Source: architecture.md#Concurrency & Resource Patterns] — Graceful shutdown via ctrlc + shutdown channel broadcast
- [Source: architecture.md#Protocol Messages] — SessionEnd 0xA3, empty payload, orchestrator → server
- [Source: architecture.md#Configuration] — No persistent config file for MVP — TUI-based setup
- [Source: epics.md#Story 3.2] — AC1 (TUI configuration), AC2 (session end), FR17, FR33
- [Source: prd.md#FR17] — User can end session by returning to keyboard and quitting
- [Source: prd.md#FR33] — Client can configure hotkey preference at startup
- [Source: 3-1-hotkey-pause-resume.md] — Code review M1: SessionEnd change reverted as out-of-scope, belongs in 3-2
- [Source: 3-1-hotkey-pause-resume.md] — Existing test patterns, setup_session() helper, pause/resume infrastructure

## Dev Agent Record

### Agent Model Used

Claude Opus 4.6 (claude-opus-4-6)

### Debug Log References

- cargo fmt required after test addition (line wrapping differences in session_handle.join().expect() chain)
- `crossbeam_channel` not available in server crate — used `std::sync::mpsc` for join timeout pattern
- `conn.send_session_end()` not usable in orchestrator main.rs because `conn` consumed by `into_split()` — used `write_orchestrator_msg` on writer directly
- `#[allow(dead_code)]` kept on connection.rs helper methods — binary crate dead_code lint doesn't count test usage

### Completion Notes List

- AC1 (TUI configuration): Fully satisfied by existing code — `tui.rs` already offers server address input (Screen 1) and hotkey selection with 9 keys (Screen 2). No code changes needed.
- AC2 (Session end): Three files modified to enable clean session teardown
- Server session.rs: SessionEnd now triggers `break` in tts_router → run_session cleanup path terminates both threads cleanly. SessionStart log updated from "not yet implemented" to "unexpected".
- Orchestrator main.rs: Removed `if !shutdown_flag` guard — SessionEnd attempted on every exit. On Ctrl+C the stream is already closed so send fails gracefully; server detects disconnect instead.
- Orchestrator connection.rs: `#[allow(dead_code)]` kept on helper methods; "Used in tests" comment on methods actually used in tests only
- 2 new integration tests: `session_end_stops_session` (orchestrator sends SessionEnd → session exits cleanly within 5s), `client_disconnect_ends_session` (TCP close → session exits cleanly within 5s)
- Both tests use mpsc channel with 5-second timeout to detect hanging threads
- 80 total tests pass (78 existing + 2 new), no regressions
- Manual E2E test: start server (with models), start orchestrator, start client. Configure hotkey in TUI (select server address, pick push-to-talk key). Start speaking, verify voice loop works. Press Ctrl+C on any process — verify all three processes exit cleanly with no zombie threads. Check `ps aux | grep space_lt` to confirm no orphaned processes.

### File List

- server/src/session.rs (MODIFIED) — SessionEnd break + SessionStart log update + 2 integration tests
- orchestrator/src/main.rs (MODIFIED) — attempt SessionEnd on exit, removed dead shutdown_flag AtomicBool, moved import to module level
- orchestrator/src/connection.rs (MODIFIED) — removed misleading "Used in tests" comment from send_session_end()

### Code Review Findings & Fixes

| ID | Severity | Description | Resolution |
|----|----------|-------------|------------|
| M1 | MEDIUM | `shutdown_flag` dead variable after guard removal (never read) | Removed AtomicBool + Arc imports entirely; shutdown is stream-based only |
| M2 | MEDIUM | "Used in tests" comment on `send_session_end()` is false | Removed misleading comment, kept plain `#[allow(dead_code)]` |
| L1 | LOW | `use` import inside function body | Moved to module-level imports |
| L2 | LOW | Comment "Always send SessionEnd" misleading on Ctrl+C path | Reworded to "Attempt to send SessionEnd" with explanation |

### Change Log

- 2026-02-21: Implemented session end handling (Story 3.2) — server handles SessionEnd from orchestrator, orchestrator always sends SessionEnd on exit, 2 integration tests for clean session teardown
- 2026-02-21: Code review fixes — removed dead shutdown_flag AtomicBool, fixed misleading comments, moved import to module level
