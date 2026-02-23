# Story 6.5: Visual Language Feedback (Corrections, Suggestions & Retry)

Status: review

## Story

As a **language learner**,
I want to see visual feedback on my terminal showing grammar corrections (in red) and naturalness suggestions (in blue) after I speak, with the option to retry my statement applying the corrections,
So that I can actively practice better phrasing through immediate repetition.

## Context

Currently, Claude's language corrections are embedded in its spoken response. This story adds a dedicated visual feedback channel: the orchestrator extracts structured feedback from Claude's output and displays it as colored text on the client terminal BEFORE the AI's spoken response plays. This provides a clear separation between pedagogical feedback (visual) and conversation (voice).

Key references:
- Story 6-3 evaluation: `_bmad-output/planning-artifacts/tts-gpu-evaluation.md` (deferred — this story is higher priority)
- Current TTS pipeline: `server/src/session.rs` (tts_router)
- Agent persona: `agent/language_trainer.agent.md`
- Protocol: `common/src/protocol.rs`

## Acceptance Criteria

1. **Given** the user speaks and STT transcribes their input
   **When** Claude detects a grammar error or unnatural phrasing
   **Then** the orchestrator extracts a `[FEEDBACK]...[/FEEDBACK]` block from Claude's response
   **And** sends it as `OrchestratorMsg::FeedbackText` to the server before `ResponseText`
   **And** the server forwards it to the client as `ServerMsg::Feedback` (no TTS synthesis)

2. **Given** the client receives a `ServerMsg::Feedback` message
   **When** the feedback contains correction lines (prefixed `RED:`)
   **Then** they are displayed in red with a cross mark symbol
   **And** when the feedback contains suggestion lines (prefixed `BLUE:`)
   **Then** they are displayed in blue with an arrow symbol
   **And** the feedback appears on the terminal BEFORE the AI's spoken response

3. **Given** the user speaks correctly or naturally enough
   **When** Claude decides no feedback is warranted
   **Then** no `[FEEDBACK]` block is included in the response
   **And** the flow is identical to the current behavior (no regression)

4. **Given** Claude's response contains a malformed `[FEEDBACK]` block (missing closing tag, etc.)
   **When** the orchestrator parses it
   **Then** the entire response is treated as spoken text (graceful degradation)
   **And** no crash or error occurs

5. **Given** the complete system
   **When** performing tests
   **Then** `parse_feedback()` has unit tests: with feedback, without, empty block, malformed, combined with `[SPEED:]`
   **And** protocol round-trip tests exist for `FeedbackText(0xA4)` and `Feedback(0x85)`
   **And** `make check` passes with all existing + new tests

## Tasks / Subtasks

- [x] Task 1: Protocol extension (AC: #1, #5)
  - [x] 1.1: Add `OrchestratorMsg::FeedbackText(String)` variant with tag `0xA4`
  - [x] 1.2: Add `ServerMsg::Feedback(String)` variant with tag `0x85`
  - [x] 1.3: Implement wire format (serialize/deserialize) for both + `ClientMsg::FeedbackChoice`, `OrchestratorMsg::FeedbackChoice`, `ServerOrcMsg::FeedbackChoice`
  - [x] 1.4: Add round-trip tests for all 5 new message types (10 tests total)

- [x] Task 2: Orchestrator feedback parsing (AC: #1, #3, #4, #5)
  - [x] 2.1: Add `parse_feedback(text: &str) -> (Option<String>, String)` function
  - [x] 2.2: Update `FORMAT_REMINDER` to mention `[FEEDBACK]` format
  - [x] 2.3: Modify `run_voice_loop` to parse feedback, send `FeedbackText`, wait for `FeedbackChoice`, handle retry with context prefix
  - [x] 2.4: Add unit tests for `parse_feedback` (7 tests: with/without, empty, malformed, with SPEED tag, empty input, leading whitespace) + 3 voice loop integration tests (feedback+continue, feedback+retry, no feedback)

- [x] Task 3: Server routing (AC: #1)
  - [x] 3.1: Handle `OrchestratorMsg::FeedbackText` in `tts_router` — forward as `ServerMsg::Feedback` to client (no TTS)
  - [x] 3.2: Handle `ClientMsg::FeedbackChoice` in `stt_router` — forward as `OrchestratorMsg::FeedbackChoice` to orchestrator

- [x] Task 4: Client display (AC: #2)
  - [x] 4.1: Handle `ServerMsg::Feedback` in `tcp_reader_loop` with stdin choice [1] Continue / [2] Retry
  - [x] 4.2: Implement `display_feedback()` with ANSI colors (red ✗ for `RED:`, blue ➜ for `BLUE:`)

- [x] Task 5: Agent prompt update (AC: #1, #3)
  - [x] 5.1: Add "Language Feedback Display" section to `language_trainer.agent.md`
  - [x] 5.2: Define format rules, frequency guidelines, interaction with existing correction approach, and examples

- [x] Task 6: Validation (AC: #5)
  - [x] 6.1: Run `make check` — all 133 tests pass (24 client + 44 common + 24 orchestrator + 41 server), zero warnings
  - [ ] 6.2: Manual E2E test: speak with deliberate errors, verify colored feedback appears before spoken response
  - [ ] 6.3: Manual E2E test: speak correctly, verify no feedback appears
  - [ ] 6.4: Manual E2E test: feedback → [2] retry → re-speak → new transcription

## Dev Notes

### Protocol Tags Allocation

| Tag | Direction | Name | Purpose |
|-----|-----------|------|---------|
| `0xA4` | Orchestrator → Server | `FeedbackText` | Language feedback (corrections + suggestions) |
| `0x85` | Server → Client | `Feedback` | Same content forwarded to client display |

### Ordering Guarantee

The orchestrator writes `FeedbackText` then `ResponseText` on the same Unix socket stream. The server's `tts_router` reads them sequentially in a single-threaded loop. The client receives `Feedback` before `Text("AI: ...")` and before any `TtsAudioChunk`. No concurrency issues.

### Feedback Format (Claude Output)

```
[FEEDBACK]
RED: "I have went to store" → "I went to the store" (past simple, not present perfect)
BLUE: "I think it is good because it has many things" → "I find it appealing for its variety" (more natural collocation)
[/FEEDBACK]
That sounds great! What else did you do yesterday?
```

### Graceful Degradation

If `parse_feedback` fails to find a valid block, the entire response goes to TTS as-is. This mirrors the `[SPEED:X.X]` pattern already proven in production.

### Files Modified

- `common/src/protocol.rs` — 5 new enum variants + wire format + tests
- `orchestrator/src/voice_loop.rs` — `parse_feedback()` + `run_voice_loop` modification + FORMAT_REMINDER update
- `orchestrator/src/connection.rs` — `ServerOrcMsg::FeedbackChoice` exhaustiveness arm in `send_session_start`
- `server/src/session.rs` — `FeedbackText` arm in `tts_router` + `FeedbackChoice` arm in `stt_router`
- `client/src/main.rs` — `Feedback` arm + `display_feedback()` + feedback writer
- `agent/language_trainer.agent.md` — New "Language Feedback Display" section
