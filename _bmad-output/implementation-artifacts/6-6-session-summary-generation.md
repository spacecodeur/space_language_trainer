# Story 6.6: Session Summary Generation

Status: ready-for-dev

## Story

As a **language learner**,
I want to generate a markdown summary of my session when I quit, covering vocabulary, errors/corrections, and grammar points discussed,
So that I can review it later to reinforce what I learned and track my progress over time.

## Context

Currently, when the user quits (Ctrl+C), the client shuts down immediately with no session recap. Claude maintains full conversation context across turns via `--continue`, which means at session end, Claude has a complete picture of everything discussed — vocabulary introduced, errors corrected, feedback given, grammar points explained, and topics covered.

This story adds an opt-in session summary: when the user presses `q` to quit, the client asks whether to generate a summary. If yes, a special request flows through the system, Claude produces a structured markdown document, and the client saves it to disk for later review.

Key references:
- Current shutdown flow: `client/src/main.rs` (Ctrl+C handler, main loop exit)
- Orchestrator voice loop: `orchestrator/src/voice_loop.rs` (run_voice_loop, LLM interaction)
- Claude backend with `--continue`: `orchestrator/src/claude.rs`
- Protocol: `common/src/protocol.rs`
- Agent persona: `agent/language_trainer.agent.md`
- Visual feedback (story 6-5): corrections and suggestions already flow through the system

## Acceptance Criteria

1. **Given** the user is in a session
   **When** they press `q` on the keyboard
   **Then** the client displays "Generate session summary? [y/n]"
   **And** waits for a single keypress (`y` or `n`)
   **And** if `n`, shuts down normally (current behavior)

2. **Given** the user presses `y` to request a summary
   **When** the request reaches the orchestrator
   **Then** the orchestrator sends a dedicated summary prompt to Claude (using `--continue` to access full session context)
   **And** the prompt asks for a structured markdown document (not voice output — no FORMAT_REMINDER)
   **And** Claude generates a summary covering: key vocabulary, errors and corrections, grammar points, and session highlights

3. **Given** the orchestrator receives Claude's summary response
   **When** the response is sent back to the client
   **Then** the client saves it as a markdown file in a configurable directory (default: `~/space-lt-sessions/`)
   **And** the filename includes the date and time (e.g., `2026-02-23_14-30.md`)
   **And** the client displays the file path before shutting down

4. **Given** the summary markdown file
   **When** the user opens it
   **Then** it contains these sections:
   - **Session Info** — date, duration, number of turns
   - **Key Vocabulary** — new or notable words/expressions used, with brief definitions or context
   - **Errors & Corrections** — the user's errors with corrections and explanations (what was said → what should have been said)
   - **Grammar Points** — grammar topics that came up (tenses, prepositions, articles, etc.), with brief reminders
   - **Session Highlights** — what went well, topics discussed, and suggested focus areas for next time

5. **Given** the summary generation is in progress
   **When** the user waits
   **Then** the client displays a progress indicator (e.g., "Generating summary...")
   **And** if the generation fails (LLM error, timeout, disconnect), the client logs the error and shuts down gracefully (no crash)

6. **Given** the complete system
   **When** performing tests
   **Then** protocol round-trip tests exist for all new message types
   **And** the summary prompt construction is unit tested
   **And** `make check` passes with all existing + new tests

## Tasks / Subtasks

- [ ] Task 1: Protocol extension (AC: #2, #6)
  - [ ] 1.1: Add `ClientMsg::SummaryRequest` variant with tag `0x06`
  - [ ] 1.2: Add `ServerMsg::SessionSummary(String)` variant with tag `0x86`
  - [ ] 1.3: Add `OrchestratorMsg::SummaryRequest` variant with tag `0xA6`
  - [ ] 1.4: Add `OrchestratorMsg::SummaryResponse(String)` variant with tag `0xA7`
  - [ ] 1.5: Add `ServerOrcMsg::SummaryRequest` variant with tag `0xA6`
  - [ ] 1.6: Implement wire format (serialize/deserialize) for all new types
  - [ ] 1.7: Add round-trip tests for all new message types

- [ ] Task 2: Client quit and summary request (AC: #1, #3, #5)
  - [ ] 2.1: Add `q` key detection in the main loop (crossterm polling alongside audio recv timeout)
  - [ ] 2.2: On `q` press, set a `quit_requested` flag (distinct from `shutdown`) to exit the main loop without closing the TCP connection
  - [ ] 2.3: After main loop exit, if `quit_requested`: prompt "Generate session summary? [y/n]" with single-keypress read (reuse `read_single_key_choice` pattern)
  - [ ] 2.4: If `y`: send `ClientMsg::SummaryRequest`, display "Generating summary...", wait for `ServerMsg::SessionSummary` with timeout
  - [ ] 2.5: Save the summary to `~/space-lt-sessions/YYYY-MM-DD_HH-MM.md`, display the path
  - [ ] 2.6: If `n` or timeout/error: proceed with normal shutdown
  - [ ] 2.7: Ensure Ctrl+C still works as immediate shutdown (no summary prompt)

- [ ] Task 3: Server routing (AC: #2)
  - [ ] 3.1: Handle `ClientMsg::SummaryRequest` in `stt_router` → forward as `OrchestratorMsg::SummaryRequest` to orchestrator
  - [ ] 3.2: Handle `OrchestratorMsg::SummaryResponse` in `tts_router` → forward as `ServerMsg::SessionSummary` to client (no TTS)

- [ ] Task 4: Orchestrator summary generation (AC: #2, #4)
  - [ ] 4.1: Add a `SUMMARY_PROMPT` constant — a structured prompt asking Claude for a markdown session summary (vocabulary, errors, grammar, highlights)
  - [ ] 4.2: Handle `ServerOrcMsg::SummaryRequest` in `run_voice_loop` (or after loop exit): call `backend.query()` with `SUMMARY_PROMPT` and `continue_session=true`
  - [ ] 4.3: Send the raw response as `OrchestratorMsg::SummaryResponse` (no feedback/speed parsing — this is markdown, not voice)
  - [ ] 4.4: Add unit test for summary prompt construction

- [ ] Task 5: Agent summary instructions (AC: #4)
  - [ ] 5.1: Add a "Session Summary Format" section to `language_trainer.agent.md` (or rely solely on the SUMMARY_PROMPT since it's a one-shot instruction at session end)

- [ ] Task 6: Validation (AC: #6)
  - [ ] 6.1: Run `make check` — all tests pass, zero warnings
  - [ ] 6.2: Manual E2E test: have a conversation, press `q`, press `y`, verify markdown file is generated
  - [ ] 6.3: Manual E2E test: press `q`, press `n`, verify normal shutdown
  - [ ] 6.4: Manual E2E test: Ctrl+C still shuts down immediately without summary prompt
  - [ ] 6.5: Verify summary file content quality (vocabulary, errors, grammar sections present)

## Dev Notes

### Protocol Tags Allocation

| Tag | Direction | Name | Purpose |
|-----|-----------|------|---------|
| `0x06` | Client → Server | `SummaryRequest` | Client requests session summary |
| `0x86` | Server → Client | `SessionSummary` | Markdown summary text |
| `0xA6` | Orchestrator ↔ Server | `SummaryRequest` | Forwarded request to orchestrator |
| `0xA7` | Orchestrator → Server | `SummaryResponse` | Claude's summary response |

### Summary Prompt Strategy

The `SUMMARY_PROMPT` is sent to Claude with `continue_session=true`, so Claude has the full conversation context. The prompt explicitly asks for markdown output (NOT voice-friendly text). The `FORMAT_REMINDER` (voice rules, [SPEED:], [FEEDBACK]) must NOT be prepended — this is a text-only generation.

Example `SUMMARY_PROMPT`:
```
Generate a detailed session summary in markdown format. Structure it with these sections:

## Session Summary — [today's date]

### Key Vocabulary
List important words and expressions that came up, with brief definitions or usage context.

### Errors & Corrections
For each significant error the user made, show:
- What was said → What should have been said (explanation)

### Grammar Points
Summarize grammar topics discussed (tenses, prepositions, articles, etc.) with brief reminders.

### Session Highlights
What the user did well, topics covered, and suggested focus areas for next time.

Output ONLY the markdown content. No preamble, no closing remarks.
```

### Quit Flow

```
User presses 'q'
  → Main audio loop exits (quit_requested=true, shutdown=false)
  → TCP connection stays open
  → Client prompts: "Generate session summary? [y/n]"
  → If 'y':
      ClientMsg::SummaryRequest → Server → OrchestratorMsg::SummaryRequest
      → Orchestrator queries Claude (--continue, SUMMARY_PROMPT)
      → OrchestratorMsg::SummaryResponse → Server → ServerMsg::SessionSummary
      → Client saves markdown file
  → Normal shutdown proceeds
```

### 'q' Key Detection

The main loop uses `audio_rx.recv_timeout(100ms)`. During the timeout window, poll crossterm for key events. The `q` key should only trigger quit when the user is NOT in listening mode (to avoid accidental quit while hotkey is active). Alternatively, detect `q` in a dedicated thread monitoring crossterm events.

### Output Directory

Default: `~/space-lt-sessions/`. Created on first use. Could later be configurable via CLI arg `--summary-dir`.

### Context Compaction Resilience

Claude Code may compact (summarize) the conversation mid-session to free context window space. A "Context Compaction" section has been added to `language_trainer.agent.md` instructing Claude to preserve specific error examples, vocabulary, grammar points, and teaching moments during compaction. Without this, Claude might discard details critical for summary generation, replacing them with generic statements like "the user made several errors." This instruction is already implemented — it lives in the system prompt sent on the first turn.

### Mini-Course Points (Future)

The user mentioned possible "mini point cours" during sessions. When/if implemented (as a separate story), they would naturally appear in Claude's conversation context and be included in the summary without changes to this feature. The `SUMMARY_PROMPT` already asks for "grammar topics discussed" which covers this.

### Files to Modify

- `common/src/protocol.rs` — 4-5 new enum variants + wire format + tests
- `client/src/main.rs` — `q` key detection, summary prompt, file saving
- `server/src/session.rs` — `SummaryRequest` routing in stt_router, `SummaryResponse` routing in tts_router
- `orchestrator/src/voice_loop.rs` — `SummaryRequest` handling, `SUMMARY_PROMPT`, Claude query
- `agent/language_trainer.agent.md` — optional: session summary format section
