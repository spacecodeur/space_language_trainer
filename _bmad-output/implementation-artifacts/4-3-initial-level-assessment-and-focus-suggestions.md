# Story 4.3: Initial Level Assessment and Focus Suggestions

Status: cancelled

> Cancelled — Claude already adapts level naturally via the Level Detection section in the agent prompt. Formal assessment and rigid focus suggestions would reduce the app's general-purpose flexibility.

## Story

As a **user**,
I want Claude to assess my English level on first use and suggest focus areas based on my weak points and NZ trip timeline,
so that my practice is targeted and efficient.

## Acceptance Criteria

1. **Given** no previous tracking files exist (first session)
   **When** the session starts
   **Then** Claude detects the absence of tracking files and initiates a conversational level assessment (FR29)
   **And** assessment covers: speaking fluency, grammar accuracy, vocabulary range, listening comprehension
   **And** after assessment, Claude estimates CEFR level and communicates it

2. **Given** tracking files exist with weak points and meta document (subsequent sessions)
   **When** the session starts
   **Then** Claude suggests focus areas based on remaining time before NZ trip (May 2026) and identified weak points (FR30)
   **And** suggestions are actionable (e.g., "You have 3 months left. Based on your recurring issues with present perfect, let's focus on that today")

3. **And** manual E2E test: first session without tracking files — verify assessment happens; subsequent session with mock tracking files — verify focus suggestions

## Tasks / Subtasks

- [ ] Task 1: Expand agent prompt with first-session assessment behavior (AC: #1)
  - [ ] 1.1: Add "First Session Behavior" section — detection of missing context, natural assessment flow
  - [ ] 1.2: Define assessment methodology — 5-10 conversational exchanges covering the 4 skill areas
  - [ ] 1.3: Define CEFR level communication — spoken estimate, positive framing, transition to normal tutoring

- [ ] Task 2: Expand agent prompt with focus suggestions for returning sessions (AC: #2)
  - [ ] 2.1: Add "Returning Session Behavior" section — how to interpret session context data
  - [ ] 2.2: Define focus suggestion format — actionable, time-aware (NZ trip countdown), based on weak points
  - [ ] 2.3: Define transition — brief suggestions then natural conversation start

- [ ] Task 3: Add session context loading in orchestrator (AC: #1, #2)
  - [ ] 3.1: Add `build_session_context()` function in `voice_loop.rs` — reads tracking files from session dir
  - [ ] 3.2: Modify `run_voice_loop()` — accept `session_dir` parameter, prepend context to first turn
  - [ ] 3.3: Update `main.rs` — pass `session_dir` to `run_voice_loop()`
  - [ ] 3.4: Add unit tests for `build_session_context()` with/without tracking files

- [ ] Task 4: Manual E2E test (AC: #3)
  - [ ] 4.1: First session (no tracking files): start full stack, verify Claude initiates assessment
  - [ ] 4.2: Returning session (mock tracking files): create `meta.md` + `weak-points.md` in session dir, verify Claude gives focus suggestions
  - [ ] 4.3: Document test results in completion notes

## Dev Notes

### CRITICAL: This Is a Prompt + Orchestrator Story

Story 4.3 has two components:
1. **Agent prompt expansion** (~30-40 lines): First-session assessment behavior and returning-session focus suggestions. Follows the same prompt-driven pattern as 4.1 and 4.2. [Source: prd.md#Design Philosophy — "Adaptive scenarios driven entirely by Claude's contextual intelligence"]
2. **Orchestrator code change** (~60-80 lines): Session context detection and first-turn context injection. The orchestrator needs to check for tracking files (`meta.md`, `weak-points.md`) in `--session-dir` and pass their content (or absence) to Claude on the first turn.

### Files to Modify

1. **`agent/language_trainer.agent.md`** (MODIFY) — Add "First Session Behavior" and "Returning Session Behavior" sections
2. **`orchestrator/src/voice_loop.rs`** (MODIFY) — Add `build_session_context()`, modify `run_voice_loop()` signature to accept `session_dir`, prepend context to first turn
3. **`orchestrator/src/main.rs`** (MODIFY) — Pass `session_dir` to `run_voice_loop()`

### Files NOT to Modify

- `orchestrator/src/claude.rs` — No changes to LlmBackend trait or Claude CLI invocation
- `server/src/*` — No server changes
- `client/src/*` — No client changes
- `common/src/protocol.rs` — No new message types needed

### CRITICAL: Agent Prompt Insertion Point

Current `language_trainer.agent.md` (180 lines) has these sections:
1. Header + core description (line 1-3)
2. Voice Output Format — CRITICAL (lines 5-17)
3. Core Persona (lines 19-26)
4. CEFR-Aware Methodology (lines 28-53)
5. Real-Time Correction Approach (lines 55-98)
6. Feedback Modes (lines 99-118)
7. Scenario Handling (lines 120-150)
8. Conversation Flow Guidelines (lines 152-158)
9. Session Sustainability (lines 160-168)
10. Boundaries (lines 170-175)
11. Final Reminder (lines 177-179)

**New sections should be inserted AFTER "CEFR-Aware Methodology" (line 53) and BEFORE "Real-Time Correction Approach" (line 55).** The assessment and focus suggestion behaviors are about session start behavior and level calibration, which logically precedes correction techniques.

Recommended insertion order:
- After line 53: new "## First Session Behavior" section
- After first session section: new "## Returning Session Behavior" section
- Then existing "Real-Time Correction Approach", "Feedback Modes", etc.

### CRITICAL: Session Context Format

The orchestrator must communicate session context to Claude via the first turn's prompt. The context is prepended before the user's actual transcribed text.

**First session (no tracking files):**
```
[Session context: This is the user's first session. No previous tracking data available.]
```

**Returning session (tracking files found):**
```
[Session context — meta.md:]
<contents of meta.md>

[Session context — weak-points.md:]
<contents of weak-points.md>

[End of session context]
```

The agent prompt instructs Claude how to interpret these markers.

**IMPORTANT:** The session context is only prepended to the FIRST turn (turn_count == 1). Subsequent turns use `--continue` which preserves the context from the first turn.

### CRITICAL: Tracking File Paths

The architecture defines the session directory structure:
```
~/language-training/
├── meta.md              # CEFR level, NZ countdown, focus areas (FR21)
├── weak-points.md       # recurring error patterns (FR22)
├── vocabulary.md        # cumulative vocabulary journal (FR23)
├── progression.md       # chronological session summaries (FR20)
└── sessions/            # per-session synthesis files (FR19)
```

For story 4-3, only check for `meta.md` and `weak-points.md` — these are the two files needed for focus suggestions. The other files are created by Epic 5 and used for other purposes.

**File detection logic:**
```rust
fn build_session_context(session_dir: &Path) -> String {
    let meta_path = session_dir.join("meta.md");
    let weak_points_path = session_dir.join("weak-points.md");

    let meta_content = std::fs::read_to_string(&meta_path).ok();
    let weak_points_content = std::fs::read_to_string(&weak_points_path).ok();

    if meta_content.is_none() && weak_points_content.is_none() {
        return "[Session context: This is the user's first session. No previous tracking data available.]".to_string();
    }

    let mut context = String::new();
    if let Some(meta) = meta_content {
        context.push_str("[Session context — meta.md:]\n");
        context.push_str(&meta);
        context.push_str("\n\n");
    }
    if let Some(wp) = weak_points_content {
        context.push_str("[Session context — weak-points.md:]\n");
        context.push_str(&wp);
        context.push_str("\n\n");
    }
    context.push_str("[End of session context]");
    context
}
```

### CRITICAL: Voice Loop Signature Change

Current `run_voice_loop` signature:
```rust
pub fn run_voice_loop(
    reader: &mut BufReader<UnixStream>,
    writer: &mut BufWriter<UnixStream>,
    backend: &dyn LlmBackend,
    agent_path: &Path,
) -> Result<()>
```

New signature adds `session_dir`:
```rust
pub fn run_voice_loop(
    reader: &mut BufReader<UnixStream>,
    writer: &mut BufWriter<UnixStream>,
    backend: &dyn LlmBackend,
    agent_path: &Path,
    session_dir: &Path,
) -> Result<()>
```

**First turn augmentation (turn_count == 1):**
```rust
let session_context = if turn_count == 1 {
    build_session_context(session_dir)
} else {
    String::new()
};

let augmented_prompt = if session_context.is_empty() {
    format!("{FORMAT_REMINDER}{text}")
} else {
    format!("{session_context}\n\n{FORMAT_REMINDER}{text}")
};
```

### CRITICAL: Agent Prompt Content — First Session Behavior

The agent prompt section should instruct Claude to:
1. Detect the `[Session context: This is the user's first session...]` marker
2. On the FIRST exchange, greet warmly and begin a natural assessment conversation
3. Over 5-10 exchanges, assess vocabulary range, grammar accuracy, fluency, and comprehension
4. Vary complexity progressively (start B1, adjust up/down based on responses)
5. After ~5-10 turns, provide a spoken CEFR estimate with positive framing
6. Transition naturally to regular tutoring at the assessed level

**IMPORTANT:** The assessment should NOT feel like a test. It should feel like a natural getting-to-know-you conversation where the user is encouraged to speak freely. Claude adjusts its questions based on the user's responses to probe different skill areas.

### CRITICAL: Agent Prompt Content — Returning Session Behavior

The agent prompt section should instruct Claude to:
1. Detect the `[Session context — meta.md:]` and `[Session context — weak-points.md:]` markers
2. Read the meta information (CEFR level, NZ trip countdown, previous focus areas)
3. Read the weak points (recurring error patterns, resolved patterns)
4. In the FIRST response, greet and briefly suggest 1-2 focus areas for today's session
5. Reference the NZ trip timeline if applicable ("You have X months until your trip...")
6. Base suggestions on unresolved weak points and previous session context
7. Keep suggestions to 2-3 sentences then start the conversation

### CRITICAL: Voice Output Format Compliance (NFR9, TTS Awareness)

All new prompt sections MUST follow the Voice Output Format rules:
- No markdown formatting in example responses
- Examples should be 1-3 spoken sentences
- Natural spoken language only
- Assessment communication must be conversational, not a formal report

The agent file must remain LLM-backend-agnostic (NFR9):
- NO references to "Claude", "Anthropic", or specific LLM names
- NO references to specific tool names like "WebSearch"
- Use generic language throughout

### CRITICAL: Existing Test Compatibility

The voice_loop.rs tests use `run_voice_loop` with the current 4-parameter signature. Adding `session_dir` requires updating ALL existing test call sites:
- `voice_loop_processes_transcription_and_sends_response`
- `voice_loop_handles_server_error_and_continues`
- `voice_loop_multi_turn_maintains_continue_flag`
- `voice_loop_sends_fallback_on_llm_error_and_continues`
- `full_orchestrator_session_with_handshake`

All tests should pass a temp directory as `session_dir` (no tracking files → first session context prepended to first turn). Verify tests still pass after the change.

### Previous Story Intelligence (from Story 4-2)

- **Story 4-2 was primarily prompt + one code constant change** — expanded agent to 171 lines / 10,225 bytes
- **Code review fixed**: added length exception for feedback summaries, improved grammar drill wording, added web search fallback, added scenario exit guidance
- **Key pattern**: `--allowedTools "WebSearch"` (not `--tools`), inline `#[cfg(test)]` modules
- **Agent file passed inline** via `cmd.args(["--system-prompt", &system_prompt])` — no file path
- **Length budget**: current file is 180 lines / ~11KB. Adding ~30-40 lines brings it to ~210-220 lines / ~13KB — well within CLI limits (128KB+)
- **E2E testing now works** with kokoro-en-v0_19 model (English-only, 330MB). TTS speed = 0.8, 8 CPU threads.
- **FORMAT_REMINDER** is prepended to every prompt in voice_loop.rs — new session context should be prepended BEFORE the FORMAT_REMINDER
- **Task 4 (manual E2E) is now feasible** — full stack testing works as of commit cda9207

### Git Intelligence (Recent Commits)

```
0018c4a Update setup.sh with TTS model download, Claude CLI check, and libclang
cda9207 Improve voice conversation quality from E2E testing feedback
b2ca1b4 Make dict_dir conditional for Kokoro models without dictionary data
81cd834 Suppress web search source citations in voice output
4f6f817 Add deferred feedback modes, scenario handling, and web search support
```

Patterns from recent commits:
- One commit per story (focused changes)
- Descriptive commit messages without "Claude" references
- `make check` used consistently for verification
- E2E improvements committed separately from story work

### Testing Strategy

1. **`make check`** — all existing tests pass (no regressions)
2. **New unit tests:**
   - `build_session_context_first_session` — empty dir → first session marker
   - `build_session_context_with_meta_only` — only meta.md present
   - `build_session_context_with_both_files` — meta.md + weak-points.md present
3. **Updated voice loop tests** — add `session_dir` parameter to all existing tests
4. **NFR9 verification** — `grep -i "claude\|anthropic" agent/language_trainer.agent.md` returns 0 matches
5. **Prompt length check** — verify file stays under ~15KB
6. **Manual E2E** — first session (assessment) + returning session (mock tracking files)

### Project Structure Notes

- Alignment with unified project structure: `agent/` for agent definitions, `orchestrator/src/` for voice loop and session management
- No new files created — only modifications to existing files
- No new dependencies needed
- Session directory detection leverages existing `--session-dir` CLI argument

### References

- [Source: epics.md#Epic 4, Story 4.3] — Acceptance criteria, FR29, FR30
- [Source: prd.md#FR29] — Initial level assessment when no previous tracking files exist
- [Source: prd.md#FR30] — Session focus suggestions based on NZ trip and weak points
- [Source: prd.md#FR27] — CEFR-adaptive conversation complexity
- [Source: architecture.md#Session Directory Structure] — ~/language-training/ with meta.md, weak-points.md, etc.
- [Source: architecture.md#Claude CLI Integration] — --system-prompt, --continue for session continuity
- [Source: architecture.md#Agent Definition Decoupling] — NFR9, LLM-backend-agnostic requirement
- [Source: 4-2-deferred-feedback-and-scenario-handling.md] — Previous story learnings, agent prompt structure, current file size
- [Source: 4-1-core-language-coaching-persona-and-real-time-feedback.md] — Voice Output Format, correction techniques, TTS awareness

## Dev Agent Record

### Agent Model Used

{{agent_model_name_version}}

### Debug Log References

### Completion Notes List

### File List
