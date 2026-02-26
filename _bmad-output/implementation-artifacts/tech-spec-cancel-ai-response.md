---
title: 'Cancel ongoing AI response'
slug: 'cancel-ai-response'
created: '2026-02-26'
status: 'implementation-complete'
stepsCompleted: [1, 2, 3, 4]
tech_stack: [crossbeam-channel, crossterm, evdev, cpal]
files_to_modify: [common/src/protocol.rs, client/src/main.rs, client/src/tui.rs, client/src/hotkey.rs, server/src/session.rs, orchestrator/src/voice_loop.rs]
code_patterns: [protocol-tag-based-messaging, atomic-flag-interruption, feedback-choice-loop, retry-context-injection, evdev-key-listener]
test_patterns: [unit-tests-in-mod, mock-llm-backend, round-trip-protocol-tests]
---

# Tech-Spec: Cancel ongoing AI response

**Created:** 2026-02-26

## Overview

### Problem Statement

When the AI produces a long or off-topic response, the user must wait for the entire TTS playback to finish. While barge-in (hotkey) can interrupt TTS audio, the AI's response and the user's last message remain in the conversation context, polluting subsequent exchanges. There is no way to signal "discard this entire exchange" either during playback or after it finishes.

### Solution

1. **Dedicated cancel key** (configurable evdev key, e.g. Escape) that can be pressed during TTS playback to immediately stop audio AND mark the exchange for cancellation.
2. **`[4] Cancel` option** in the post-response feedback menu for cancellation after TTS finishes.
3. **New `CancelExchange` protocol message** (tag 0x07 client, 0xA9 orchestrator) propagated from client → server → orchestrator.
4. **Orchestrator context reset**: inject a cancel context prompt at the next conversational turn so Claude treats the previous exchange as cancelled.

### Scope

**In Scope:**
- New configurable cancel key in client (evdev-based, separate listener)
- `[4] Cancel` option in feedback menu (crossterm-based)
- New `CancelExchange` protocol message (client → server → orchestrator)
- Server routing of cancel message from STT router to orchestrator
- Orchestrator cancel handling: inject reset prompt on next turn
- Unit tests for all new components

**Out of Scope:**
- Modifying Claude CLI API (no real context rollback possible)
- Cancelling during LLM query phase (before TTS starts)
- Multi-exchange undo (only the last exchange)
- Changing the existing barge-in/InterruptTts mechanism

## Context for Development

### Codebase Patterns

- **Protocol message system**: Tag-based binary protocol in `common/src/protocol.rs`. Client tags 0x01-0x06 used, next free: 0x07. Orchestrator tags 0xA0-0xA8 used, next free: 0xA9. Each message needs: enum variant + `write_*_msg` arm + `read_*_msg` arm + round-trip test. `ServerOrcMsg` (lines 62-68) is the subset used on the Unix socket between server and orchestrator.
- **Evdev hotkey system**: `client/src/hotkey.rs:38-90` — `listen_all_keyboards(key, is_listening)` spawns one thread per keyboard device, monitors a single `KeyCode`, toggles an `AtomicBool`. For cancel, a separate `listen_cancel_key()` function sets (not toggles) an AtomicBool on press.
- **Feedback choice loop**: `client/src/main.rs:808-820` — `FeedbackAction` enum (Continue, Retry, Replay) at line 515. `read_feedback_choice()` at lines 523-558 reads single keypress via crossterm ('1','2','3'). Sends `FeedbackChoice(bool)` to server.
- **Message forwarding (server)**: `session.rs:191-196` forwards `FeedbackChoice` directly from client → orchestrator. Same one-line pattern for SummaryRequest (lines 198-200).
- **Retry context injection**: `voice_loop.rs:249-252` sets `retry_context = Some("...")`. Consumed at lines 157-161 via `.take()` and prepended to next prompt. Cancel context follows this exact pattern.
- **Orchestrator feedback wait**: `voice_loop.rs:219-239` blocks on `read_server_orc_msg()`, only handles `FeedbackChoice`. Must also handle `CancelExchange` in this loop.
- **Shared atomic flags**: Main loop ↔ tcp_reader_loop share `is_playing`, `playback_clear` via `Arc<AtomicBool>`. Same pattern for `cancel_requested`.

### Files to Reference

| File | Purpose | Key Lines |
| ---- | ------- | --------- |
| common/src/protocol.rs | Protocol messages, ser/deser | ClientMsg 18-28, OrchestratorMsg 44-57, ServerOrcMsg 62-68, write_client_msg 72-111, read_client_msg 113-170 |
| client/src/main.rs | Feedback loop, barge-in, TTS control | Shared flags 98-103, hotkey init 129-131, barge-in 243-256, FeedbackAction 515-519, read_feedback_choice 523-558, tcp_reader_loop TtsEnd 766-788, feedback loop 804-829 |
| client/src/tui.rs | Setup config, key selection | SetupConfig 16-22, hotkey choices 47-98, select_screen 164-208 |
| client/src/hotkey.rs | Evdev key listener threads | listen_all_keyboards 38-90, key event handling 66-84 |
| server/src/session.rs | STT/TTS routers, message forwarding | STT router 127-206, InterruptTts 187-189, FeedbackChoice forwarding 191-196 |
| orchestrator/src/voice_loop.rs | FSM, retry context, feedback handling | States 45-50, retry_context 96/157/249-252, feedback wait 219-239, ResponseText send 245/264 |

### Technical Decisions

- **Separate cancel key listener**: A new `listen_cancel_key()` function rather than modifying `listen_all_keyboards()`. Different semantics: hotkey toggles a bool, cancel sets a one-shot flag. Spawns its own threads on the same keyboard devices (evdev allows multiple readers).
- **One-shot AtomicBool for cancel**: `cancel_pressed` is set to `true` by the listener, cleared by the main loop after handling. Simpler than a channel; no risk of missing events at the polling rate (10ms audio chunks).
- **cancel_requested shared flag**: Bridges main loop (sets it on cancel key) and tcp_reader_loop (checks it at TtsEnd to skip feedback). Same pattern as `is_playing` and `playback_clear`.
- **Cancel during TTS = InterruptTts + CancelExchange**: Two messages sent. InterruptTts stops audio (existing mechanism). CancelExchange signals the orchestrator.
- **Cancel from feedback menu = CancelExchange only**: TTS already finished. No FeedbackChoice sent — the orchestrator sees CancelExchange instead and handles it.
- **Cancel context injection**: Same mechanism as retry_context. Prompt tells Claude to ignore the previous exchange. Both retry_context and cancel_context can coexist as separate Options — if cancel is set, it takes priority.
- **Feedback menu skip on cancel**: When cancel is triggered during TTS and TtsEnd arrives, tcp_reader_loop checks `cancel_requested` and skips the feedback menu. The user goes directly back to listening.

## Implementation Plan

### Tasks

- [ ] Task 1: Add CancelExchange to protocol
  - File: `common/src/protocol.rs`
  - Action:
    1. Add `CancelExchange` variant to `ClientMsg` enum (after FeedbackChoice)
    2. Add tag `0x07` to `write_client_msg` — empty payload (same pattern as InterruptTts at lines 93-96)
    3. Add tag `0x07` to `read_client_msg` — return `ClientMsg::CancelExchange` (same pattern as InterruptTts at lines 148-153)
    4. Add `CancelExchange` variant to `OrchestratorMsg` enum (after StatusNotification)
    5. Add tag `0xA9` to `write_orchestrator_msg` — empty payload
    6. Add tag `0xA9` to `read_orchestrator_msg`
    7. Add `CancelExchange` to `ServerOrcMsg` enum (line 62-68) — needed for Unix socket routing
    8. Add `0xA9` handling to `write_server_orc_msg` and `read_server_orc_msg`
    9. Add round-trip tests: `round_trip_cancel_exchange` (client) and orchestrator variant
  - Notes: Follow exact serialization pattern of InterruptTts (tag + 0-length payload). ServerOrcMsg is the key bridge — it must include CancelExchange for server↔orchestrator routing.

- [ ] Task 2: Add cancel key listener to client
  - File: `client/src/hotkey.rs`
  - Action:
    1. Add `pub fn listen_cancel_key(key: KeyCode, cancel_flag: Arc<AtomicBool>) -> Result<()>` — same structure as `listen_all_keyboards` but SETS flag to true on key press (not toggle). One-shot: `cancel_flag.store(true, Ordering::SeqCst)`.
    2. Reuse `find_keyboards()` (already exists, no changes needed).
  - File: `client/src/tui.rs`
  - Action:
    1. Add `cancel_key: EvdevKeyCode` field to `SetupConfig` (line 21, after `hotkey`)
    2. Add Screen 2b after hotkey selection: "Select Cancel Key" with choices (Escape, F5, F6, F7, F8). Must not overlap with hotkey choices. Default: Escape.
    3. Map selection index to `EvdevKeyCode` (e.g., 0 → KEY_ESC, 1 → KEY_F5, etc.)
  - Notes: Keep hotkey and cancel key as non-overlapping sets. Escape is the natural cancel key. The listener threads are lightweight (blocked on evdev read).

- [ ] Task 3: Add cancel during TTS playback in client
  - File: `client/src/main.rs`
  - Action:
    1. Add `cancel_requested: Arc<AtomicBool>` alongside `is_playing` and `playback_clear` (near line 102). Clone for tcp_reader_loop.
    2. Add `cancel_pressed: Arc<AtomicBool>` for the evdev listener (near line 130). Call `hotkey::listen_cancel_key(config.cancel_key, cancel_pressed.clone())`.
    3. In the main audio/VAD loop (after the barge-in check at line 256), add cancel key check:
       ```
       if cancel_pressed.compare_exchange(true, false, SeqCst, SeqCst).is_ok() {
           if is_playing.load(SeqCst) {
               info!("[CANCEL] Cancel key pressed during TTS");
               // Stop TTS audio
               write_client_msg(&mut writer, &ClientMsg::InterruptTts)?;
               // Signal cancellation to orchestrator
               write_client_msg(&mut writer, &ClientMsg::CancelExchange)?;
               is_playing.store(false, SeqCst);
               playback_clear.store(true, SeqCst);
               cancel_requested.store(true, SeqCst);
           }
       }
       ```
    4. Pass `cancel_requested` clone to `tcp_reader_loop` (add parameter).
    5. In `tcp_reader_loop` TtsEnd handler (line 766): after flushing resampler, check `cancel_requested`. If true, clear it, skip the rest (don't show replay hint, don't enter feedback loop). Just set `is_playing = false` and continue.
  - Notes: `compare_exchange` ensures we consume the flag atomically (no double handling). Cancel during non-TTS does nothing (guard on `is_playing`). The `cancel_requested` flag tells tcp_reader_loop to skip feedback display when TtsEnd arrives.

- [ ] Task 4: Add [4] Cancel to feedback menu
  - File: `client/src/main.rs`
  - Action:
    1. Add `Cancel` variant to `FeedbackAction` enum (line 518)
    2. In `read_feedback_choice` (line 546-550): add `KeyCode::Char('4') => FeedbackAction::Cancel` and `KeyCode::Esc => FeedbackAction::Cancel`
    3. In fallback stdin mode (line 531-534): add `"4" => FeedbackAction::Cancel`
    4. Update feedback menu display (line 809): `"[1] Continue  [2] Retry and re-speak  [3] Replay  [4] Cancel"`
    5. In the feedback loop match (lines 813-819): add `FeedbackAction::Cancel => break` with a special return value. Instead of `proceed: bool`, use an enum or Option to distinguish cancel from continue/retry.
    6. After the feedback loop: if cancel, send `CancelExchange` instead of `FeedbackChoice`. Skip sending FeedbackChoice entirely.
  - Notes: The feedback loop currently returns a `bool` (proceed). Change to return a 3-way result: Continue(true), Retry(false), or Cancel. Use an enum `FeedbackResult { Continue, Retry, Cancel }` or `Option<bool>` where None = cancel. After the loop, match on the result: Continue/Retry → send FeedbackChoice as before, Cancel → send CancelExchange.

- [ ] Task 5: Route CancelExchange in server
  - File: `server/src/session.rs`
  - Action:
    1. In STT router's client message match (near lines 187-200), add:
       ```
       ClientMsg::CancelExchange => {
           info!("[server] CancelExchange received, forwarding to orchestrator");
           write_orchestrator_msg(&mut writer, &OrchestratorMsg::CancelExchange)?;
       }
       ```
    2. In TTS router's orchestrator message match (if needed): no action required — CancelExchange is routed via the STT router → Unix socket path, not the TTS router.
  - Notes: Follows the exact same pattern as FeedbackChoice forwarding (lines 191-196) and SummaryRequest forwarding (lines 198-200). One-line forwarding.

- [ ] Task 6: Handle CancelExchange in orchestrator
  - File: `orchestrator/src/voice_loop.rs`
  - Action:
    1. Add `cancel_context: Option<String>` (near `retry_context` at line 96). Initial value: `None`.
    2. Define cancel context string: `"[The user cancelled the previous exchange. Discard it entirely — do not reference it, quote it, or follow up on it. Treat the next message as a fresh conversational turn.]\n\n"`
    3. In the feedback wait loop (lines 219-239), add a match arm for `ServerOrcMsg::CancelExchange`:
       ```
       ServerOrcMsg::CancelExchange => {
           info!("[orchestrator] Exchange cancelled by user");
           cancel_context = Some(CANCEL_CONTEXT.to_string());
           state = VoiceLoopState::WaitingForTranscription;
           continue 'outer;  // Break out of feedback wait, back to main loop
       }
       ```
       This skips sending ResponseText — the AI response is discarded.
    4. In the main message loop, add handling for CancelExchange when in `WaitingForTranscription` state (for the no-feedback path where orchestrator already sent ResponseText):
       ```
       ServerOrcMsg::CancelExchange => {
           info!("[orchestrator] Exchange cancelled by user (post-response)");
           cancel_context = Some(CANCEL_CONTEXT.to_string());
       }
       ```
    5. In the prompt augmentation (lines 157-161), apply cancel_context with priority over retry_context:
       ```
       let augmented_prompt = if let Some(ctx) = cancel_context.take() {
           format!("{FORMAT_REMINDER}{ctx}{text}")
       } else if let Some(ctx) = retry_context.take() {
           format!("{FORMAT_REMINDER}{ctx}{text}")
       } else {
           format!("{FORMAT_REMINDER}{text}")
       };
       ```
  - Notes: Two reception paths because the orchestrator can be in different states when CancelExchange arrives: (a) blocked in feedback wait loop (feedback present, TTS interrupted) or (b) back in main loop (no feedback, already sent ResponseText). Both set cancel_context for the next turn. The `continue 'outer` requires labeling the outer voice loop — currently unlabeled (line 97: `loop {`), must add `'outer: loop {`.

- [ ] Task 7: Add unit tests
  - File: `common/src/protocol.rs`
  - Action: Add `round_trip_cancel_exchange` test — write + read ClientMsg::CancelExchange, verify round-trip. Same for OrchestratorMsg variant if not already covered by existing test pattern.
  - File: `orchestrator/src/voice_loop.rs`
  - Action: Add `voice_loop_cancel_injects_context` test — simulate: send TranscribedText, mock LLM returns response, send CancelExchange instead of FeedbackChoice, then send another TranscribedText. Verify the second LLM query prompt contains the cancel context string.
  - File: `server/src/session.rs`
  - Action: Add `cancel_exchange_forwarded_to_orchestrator` test — verify that ClientMsg::CancelExchange received on TCP is forwarded as OrchestratorMsg::CancelExchange on Unix socket.
  - Notes: Follow existing test patterns. Orchestrator test uses MockLlmBackend. Server test uses the existing TCP↔Unix socket test harness.

### Acceptance Criteria

- [ ] AC1: Given the client is playing TTS audio, when the user presses the configured cancel key, then TTS audio stops immediately AND a CancelExchange message is sent to the orchestrator.
- [ ] AC2: Given the client displays the feedback menu after TTS, when the user presses '4' or Escape, then a CancelExchange message is sent to the orchestrator (no FeedbackChoice is sent).
- [ ] AC3: Given the orchestrator receives CancelExchange during the feedback wait, when the next user utterance is transcribed, then the LLM query includes a cancel context prompt instructing Claude to ignore the previous exchange.
- [ ] AC4: Given the orchestrator receives CancelExchange after already sending ResponseText (no-feedback path), when the next user utterance is transcribed, then the LLM query includes the cancel context prompt.
- [ ] AC5: Given the user presses the cancel key during TTS, when TtsEnd is received by the client, then the feedback menu is NOT displayed and the client returns directly to listening state.
- [ ] AC6: Given cancel is triggered, when the orchestrator sends the cancelled AI response text, then it is NOT sent to TTS (feedback path) OR already sent but the user won't hear it because TTS was interrupted (no-feedback path).
- [ ] AC7: Given `make check` is run, when all tests execute, then all existing tests still pass and new tests (protocol round-trip, orchestrator cancel context, server forwarding) pass.

## Additional Context

### Dependencies

- No new crate dependencies needed
- Uses existing: evdev (hotkey), crossterm (feedback keys), crossbeam-channel (communication), protocol infrastructure (common)

### Testing Strategy

- **Unit tests** (automated):
  - Protocol round-trip for CancelExchange (common/src/protocol.rs)
  - Orchestrator cancel context injection with MockLlmBackend (orchestrator/src/voice_loop.rs)
  - Server CancelExchange forwarding (server/src/session.rs)
- **Regression tests** (automated):
  - All existing tests must pass unchanged (154+ tests)
- **Manual testing**:
  - Play a multi-sentence TTS response, press cancel key mid-playback → verify audio stops and next interaction doesn't reference cancelled exchange
  - Let TTS finish, press [4] Cancel in feedback menu → verify same behavior
  - Verify barge-in (hotkey) still works independently of cancel
  - Verify [1] Continue and [2] Retry still work unchanged

### Notes

- **Risk — Claude context accumulation**: The cancel context prompt adds ~30 tokens to the next turn. If the user cancels frequently, these prompts accumulate in Claude's session. This is acceptable — Claude CLI manages its own context window, and the cancel prompts are small.
- **Risk — Race condition**: CancelExchange and InterruptTts are sent as two separate messages. The server processes them in order (same TCP stream, same STT router thread). No race condition possible.
- **Risk — Cancel key same as hotkey**: Setup prevents overlap by using non-overlapping key sets (hotkey: F2-F4, F9-F12, ScrollLock, Pause; cancel: Escape, F5-F8). If the user manually configures the same key, the hotkey toggle takes precedence (processed first in main loop).
- **Future consideration**: Could add cancel during LLM query phase (before TTS), but this requires killing the Claude CLI process mid-query, which risks session corruption. Out of scope for now.
- **Limitation**: Cancel doesn't truly erase the exchange from Claude's context — it injects a "pretend it didn't happen" instruction. For most conversational purposes this is sufficient.
