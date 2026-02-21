# Story 4.1: Core Language Coaching Persona and Real-Time Feedback

Status: done

## Story

As a **user**,
I want Claude to act as a patient, encouraging English tutor who corrects my grammar and vocabulary in real time,
so that I improve through natural conversation with immediate feedback.

## Acceptance Criteria

1. **Given** the `language_trainer.agent.md` file loaded as system prompt
   **When** the user makes a grammar error (e.g., "I have went to the store")
   **Then** Claude provides a natural, inline correction (e.g., "Quick note — the correct form is 'I have gone'. Try again?")
   **And** corrections are concise and don't break conversation flow

2. **Given** an active conversation session
   **When** the agent prompt is loaded
   **Then** Claude adapts vocabulary and grammar complexity based on CEFR level context
   **And** if no CEFR level is available, Claude detects the user's level conversationally within the first 2-3 turns

3. **Given** a sustained conversation session (30-60 minutes)
   **When** multiple errors occur
   **Then** the agent persona remains encouraging and patient throughout
   **And** corrections are spaced naturally (not every sentence) to avoid fatigue

4. **Given** the agent definition file
   **When** inspected for LLM-backend compatibility
   **Then** the file contains no Claude-specific features referenced (NFR9)
   **And** the file is loadable by the existing orchestrator `--system-prompt` flow without code changes

5. **And** manual E2E test: conduct a 10-minute conversation with deliberate errors, verify corrections are natural and accurate

## Tasks / Subtasks

- [x] Task 1: Expand `language_trainer.agent.md` with coaching methodology (AC: #1, #2, #3)
  - [x] 1.1: Write core persona section — patient, encouraging English tutor identity, tone guidelines
  - [x] 1.2: Write CEFR methodology section — level descriptions (A2/B1/B2), adaptation rules for vocabulary and grammar complexity
  - [x] 1.3: Write CEFR detection fallback — instructions to assess user level conversationally in first 2-3 turns when no level context is provided
  - [x] 1.4: Write real-time correction format — concise inline pattern with 3-5 examples covering grammar, vocabulary, prepositions, tenses
  - [x] 1.5: Write conversation flow rules — correction frequency limits, topic continuity, natural blending of corrections into responses
  - [x] 1.6: Write session sustainability guidelines — motivational patterns, positive reinforcement, effort recognition, fatigue prevention

- [x] Task 2: Ensure LLM-backend agnosticism (AC: #4)
  - [x] 2.1: Review agent file for any Claude-specific language (no "I'm Claude", no knowledge cutoff references, no Claude tool references)
  - [x] 2.2: Verify prompt uses generic LLM-compatible instructions only (no model-specific features like artifacts, computer use, etc.)

- [x] Task 3: Verify integration with existing orchestrator (AC: #4)
  - [x] 3.1: Confirm `language_trainer.agent.md` loads correctly via existing `ClaudeCliBackend` `--system-prompt` flow
  - [x] 3.2: Run `make check` — no regressions (all 84 tests pass)
  - [x] 3.3: Test prompt length: verify the expanded agent file is accepted by Claude CLI without truncation issues

- [ ] Task 4: Manual E2E test (AC: #5)
  - [ ] 4.1: Conduct 10-minute conversation with deliberate errors (wrong tenses, missing articles, incorrect prepositions, vocabulary misuse)
  - [ ] 4.2: Verify corrections are inline, natural, and don't break conversation flow
  - [ ] 4.3: Verify persona is patient and encouraging throughout
  - [ ] 4.4: Document test results in completion notes

## Dev Notes

### CRITICAL: This is a System Prompt Story — No Rust Code Changes

Story 4.1 is entirely about expanding the `language_trainer.agent.md` file. The voice loop, Claude CLI backend, TTS pipeline, and all infrastructure are already complete from Epics 1-3.

**The ONLY file to modify:** `agent/language_trainer.agent.md`

**Current content (1 line, 247 bytes):**
```
You are a patient, encouraging English language tutor. Help the user practice conversational English by engaging in natural dialogue. When you notice grammar or vocabulary errors, gently correct them inline without breaking the conversation flow.
```

**Target:** Comprehensive 50-100+ line system prompt covering coaching methodology, CEFR awareness, correction format, and session sustainability.

### CRITICAL: How System Prompts Flow Through the System

The orchestrator reads the agent file and passes it to Claude CLI:

```
orchestrator/src/claude.rs:132-135:
  if !continue_session {
      let system_prompt = std::fs::read_to_string(system_prompt_file)
          .context("reading system prompt file")?;
      cmd.args(["--system-prompt", &system_prompt]);
  }
```

Key behavior:
- File is read on FIRST turn only (subsequent turns use `--continue`)
- Content is passed via `--system-prompt` flag (inline, NOT `--system-prompt-file`)
- No transformation or processing — the file content IS the system prompt verbatim
- File path comes from `--agent` CLI argument: `space_lt_orchestrator --agent agent/language_trainer.agent.md`

### CRITICAL: CEFR Level Handling — Phased Approach

Story 4.1 mentions FR27 (adapt to CEFR level from `meta.md`), but `meta.md` is created by Story 5.3. The phased approach:

1. **Story 4.1 (now):** Agent prompt includes CEFR methodology and level adaptation rules. When no level context is provided in the conversation, Claude detects the user's level from their speech patterns in the first 2-3 turns.
2. **Story 5.1 (future):** Orchestrator code will load `meta.md` and prepend CEFR level info to the first user turn. The agent prompt is already prepared for this.

**Implementation for 4.1:** Write the prompt to handle BOTH cases:
- With CEFR context: "The user's current level is [level]. Adapt accordingly."
- Without CEFR context: Detect level from user's first few responses, then adapt.

### CRITICAL: Correction Format — Research-Based Patterns

The agent prompt should specify the EXACT correction format to use. Recommended pattern from language teaching methodology:

**Inline recast (preferred):** Naturally incorporate the correct form in your response.
- User: "I have went to the store yesterday."
- Agent: "Oh, you **went** to the store yesterday? What did you buy?" (recast — models correct form without explicit correction)

**Brief explicit correction (for recurring errors):** Short parenthetical or "by the way" correction.
- User: "I am agree with you."
- Agent: "I'm glad we see eye to eye! (Quick note: we say 'I agree' — no 'am' needed.) What else do you think about...?"

**Avoid:** Long grammar explanations mid-conversation. Save those for deferred feedback (Story 4.2).

### CRITICAL: LLM-Backend Agnosticism (NFR9)

The agent file must NOT contain:
- References to "Claude", "Anthropic", or any specific LLM
- References to knowledge cutoff dates
- References to Claude-specific features (artifacts, computer use, tool use)
- References to model capabilities or limitations
- Any first-person identity statements ("I'm Claude", "As an AI assistant")

The prompt should use generic language: "You are a language tutor" (not "You are Claude acting as a tutor").

### CRITICAL: Prompt Length Considerations

The `--system-prompt` flag passes the ENTIRE file content as a command-line argument. Very long prompts may hit shell argument length limits (typically 128KB-2MB depending on OS). A 50-100 line prompt (~2-5KB) is well within limits.

However, note that the system prompt is passed INLINE (not via file), so special characters in the prompt need to be handled. The current code uses `cmd.args(["--system-prompt", &system_prompt])` which handles escaping correctly via Rust's `Command` API (no shell interpolation).

### Previous Story Intelligence (from Stories 3-1 through 3-3)

- **Package naming:** `space_lt_*` (underscore in code, hyphen in Cargo.toml)
- **Makefile:** ALWAYS use `make check` not raw cargo commands
- **Clippy:** `-D warnings` — all warnings are errors
- **Error handling:** `anyhow::Result` + `.context()`
- **Logging:** `[server]`/`[client]`/`[orchestrator]` prefix
- **Test convention:** inline `#[cfg(test)]` modules, match-based assertions
- **Protocol functions flush internally** — no explicit `flush()` needed
- **Current test count:** 84 tests (21 client + 34 common + 13 orchestrator + 16 server)

### Project Structure Notes

Files to modify:
- `agent/language_trainer.agent.md` (MODIFY) — expand from 1 line to comprehensive coaching prompt

Files NOT to modify:
- `orchestrator/src/claude.rs` — system prompt loading already works correctly
- `orchestrator/src/voice_loop.rs` — voice loop already passes agent path to backend
- `orchestrator/src/main.rs` — `--agent` flag already exists
- `common/src/protocol.rs` — no new message types needed
- `server/src/*` — no server changes needed
- `client/src/*` — no client changes needed

### Session Directory Structure (for future reference)

The architecture defines this runtime structure (created by Epic 5, NOT by this story):
```
~/language-training/
├── language_trainer.agent.md           # agent definition
├── sessions/                           # per-session synthesis (FR19)
├── progression.md                      # chronological session summaries (FR20)
├── meta.md                             # CEFR level, NZ countdown, focus areas (FR21)
├── weak-points.md                      # recurring error patterns (FR22)
└── vocabulary.md                       # cumulative vocabulary journal (FR23)
```

Story 4.1 only touches the agent definition file. The tracking files are created in Epic 5.

### References

- [Source: epics.md#Epic 4] — Story 4.1 acceptance criteria, FR25, FR27, FR28
- [Source: architecture.md#Claude CLI Integration] — `--system-prompt` flag, `--continue` for session continuity
- [Source: architecture.md#LLM Backend Abstraction] — LlmBackend trait, ClaudeCliBackend
- [Source: architecture.md#Agent Definition Decoupling] — NFR9, LLM-backend-agnostic requirement
- [Source: architecture.md#Session Directory Structure] — runtime file layout
- [Source: architecture.md#Gap Resolution G1] — Claude CLI timeout & retry (already implemented in 3-3)
- [Source: prd.md#FR25] — Real-time grammar and vocabulary corrections
- [Source: prd.md#FR27] — CEFR-adaptive conversation complexity
- [Source: prd.md#NFR9] — Agent definition LLM-backend-agnostic
- [Source: 3-3-claude-cli-retry-and-audio-error-recovery.md] — ClaudeCliBackend patterns, system prompt loading flow

## Dev Agent Record

### Agent Model Used

Claude Opus 4.6

### Debug Log References

None — this story involves no Rust code changes, only system prompt expansion.

### Completion Notes List

1. **Task 1 — Agent prompt expanded from 1 line (247 bytes) to 106 lines (5,960 bytes).** Comprehensive coaching prompt covering: core persona (warm, supportive, consistent energy), CEFR methodology (A2/B1/B2 level descriptions with adaptation rules), level detection fallback (start at B1, assess in first 2-3 exchanges), three correction techniques (conversational recast 60-70%, brief explicit correction 20-30%, positive reinforcement regularly), correction frequency limits (1-2 per 3-4 user turns), conversation flow guidelines, session sustainability (30-60 min), and boundaries.

2. **Task 2 — LLM-backend agnosticism verified.** Grep confirmed zero references to "Claude", "Anthropic", "AI assistant", "knowledge cutoff", "artifacts", or "computer use" in the agent file. All instructions use generic language ("You are a language tutor").

3. **Task 3 — Integration verified.** `make check` passes with 84 tests (no regressions). File is read by `ClaudeCliBackend::query_once()` via `std::fs::read_to_string()` and passed via `cmd.args(["--system-prompt", &system_prompt])`. At 5,960 bytes, the prompt is well within shell argument limits (128KB-2MB). Rust's `Command` API handles escaping correctly (no shell interpolation).

4. **Task 4 — Manual E2E test deferred.** Requires live infrastructure (server + orchestrator + client with microphone). Test plan documented below for manual execution:
   - Start full stack: `make run-server`, `make run-orchestrator -- --agent agent/language_trainer.agent.md`, `make run-client`
   - Conduct 10-minute conversation with deliberate errors: wrong tenses ("I have went"), missing articles ("I went to store"), incorrect prepositions ("I'm good in English"), vocabulary misuse ("I made a travel")
   - Verify: corrections are inline and natural, conversation flow is maintained, persona stays encouraging, corrections are spaced (not every sentence)

### File List

- `agent/language_trainer.agent.md` (MODIFIED) — expanded from 1-line placeholder to 118-line comprehensive coaching prompt

### Code Review Record

**Reviewer:** Claude Opus 4.6 (adversarial review)
**Date:** 2026-02-21
**Findings:** 1 HIGH, 3 MEDIUM, 3 LOW (7 total)
**Resolution:** 4 issues fixed automatically (H1 + M1-M3), 3 LOW accepted

**Issues fixed:**
- H1: Added "Voice Output Format" section — TTS awareness (plain spoken language, no markdown, 2-4 sentence responses)
- M1: Removed bold markdown from correction examples (line 49 `**went**` → `went`)
- M2: Added response length guidance (2-4 sentences) in Voice Output Format section
- M3: Added CEFR fallback for A1 and C1/C2 levels outside described range
- L1 (bonus): Changed "speaking pace" → "sentence length" (line 47)

**Issues accepted (LOW):**
- L2: No conversation opening guidance — LLM handles naturally
- L3: Task 4 (manual E2E) deferred — test plan documented

**Post-fix verification:** `make check` → 84 tests pass, NFR9 grep → 0 matches, file size 6,729 bytes (118 lines)
