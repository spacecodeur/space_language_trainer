# Story 4.2: Deferred Feedback and Scenario Handling

Status: done

## Story

As a **user**,
I want to switch between real-time and deferred feedback modes, and request different practice scenarios vocally,
so that I can tailor each session to my learning needs.

## Acceptance Criteria

1. **Given** an active conversation with real-time feedback (default)
   **When** the user says "let's switch to deferred feedback" or similar vocal request
   **Then** Claude acknowledges and stops inline corrections, saving them for session summary
   **And** the user can switch back to real-time feedback vocally

2. **Given** an active conversation
   **When** the user requests a scenario vocally (e.g., "let's do an interview simulation", "can we practice grammar?", "let's discuss a topic")
   **Then** Claude seamlessly transitions to the requested scenario without formal mode switching
   **And** the following scenario types are supported: free conversation, grammar drills, interview simulation, topic discussion with web search, level assessment (FR28)

3. **Given** a topic discussion scenario
   **When** the user requests a specific topic (e.g., "let's discuss climate change")
   **Then** web search is used to provide current information for the discussion (FR12)
   **And** web search happens without requiring user approval

4. **And** manual E2E test: switch feedback modes vocally, request 3 different scenarios, verify smooth transitions

## Tasks / Subtasks

- [x] Task 1: Expand `language_trainer.agent.md` with deferred feedback mode (AC: #1)
  - [x] 1.1: Add "Feedback Modes" section — define real-time (default) vs deferred mode behavior
  - [x] 1.2: Write mode switching instructions — vocal triggers, acknowledgment format, switching back
  - [x] 1.3: Write deferred correction tracking — how to internally note errors and present a summary when requested or at session end

- [x] Task 2: Expand `language_trainer.agent.md` with scenario handling (AC: #2)
  - [x] 2.1: Add "Scenario Handling" section — introduce the 5 scenario types with transition guidelines
  - [x] 2.2: Write free conversation scenario — default mode, natural flowing dialogue (already partially covered by 4.1)
  - [x] 2.3: Write grammar drills scenario — structured exercises, targeted practice patterns
  - [x] 2.4: Write interview simulation scenario — role-play instructions, professional context
  - [x] 2.5: Write topic discussion scenario — web search integration instructions, how to weave search results into conversation
  - [x] 2.6: Write level assessment scenario — structured assessment approach, CEFR evaluation criteria

- [x] Task 3: Enable web search in Claude CLI invocation (AC: #3)
  - [x] 3.1: Update `orchestrator/src/claude.rs` — change `--tools ""` to allow web search tool
  - [x] 3.2: Add unit test verifying the command construction includes the web search tool flag
  - [x] 3.3: Run `make check` — no regressions (all tests pass)

- [ ] Task 4: Manual E2E test (AC: #4)
  - [ ] 4.1: Switch between real-time and deferred feedback vocally, verify acknowledgment
  - [ ] 4.2: Request 3 different scenarios (grammar drill, interview sim, topic discussion), verify transitions
  - [ ] 4.3: Verify web search triggers during topic discussion
  - [ ] 4.4: Document test results in completion notes

## Dev Notes

### CRITICAL: This Is Primarily a System Prompt Story — Minimal Code Change

Story 4.2 is ~90% agent prompt expansion (like 4.1) plus one targeted code change to enable web search. The deferred feedback mode and scenario handling are behavioral changes driven by the system prompt — the architecture explicitly states: "Adaptive scenarios driven entirely by Claude's contextual intelligence — no formal scenario engine required" [Source: prd.md#Design Philosophy].

### Files to Modify

1. **`agent/language_trainer.agent.md`** (MODIFY) — Add deferred feedback mode and scenario handling sections
2. **`orchestrator/src/claude.rs`** (MODIFY) — Enable web search tool in Claude CLI invocation (1 line change)

### Files NOT to Modify

- `orchestrator/src/voice_loop.rs` — Voice loop just passes text back and forth; doesn't care about feedback mode or scenarios
- `orchestrator/src/main.rs` — No new CLI flags needed
- `server/src/*` — No server changes (TTS/STT pipeline unchanged)
- `client/src/*` — No client changes
- `common/src/protocol.rs` — No new message types needed

### CRITICAL: Web Search Code Change (Task 3)

**Current state** (`orchestrator/src/claude.rs:138`):
```rust
cmd.args(["--output-format", "text", "--tools", ""]);
```

This explicitly disables ALL Claude CLI tools. For FR12 (web search during topic discussions), we need to enable the web search tool.

**Required change:** Replace `--tools ""` with flags that allow web search while keeping destructive tools (Bash, Edit, Write, etc.) disabled. The Claude CLI supports `--allowedTools` to whitelist specific tools.

**Approach:**
```rust
// Before:
cmd.args(["--output-format", "text", "--tools", ""]);

// After:
cmd.args(["--output-format", "text", "--allowedTools", "WebSearch"]);
```

**IMPORTANT:** The dev MUST verify the exact Claude CLI flag name and tool name:
- Run `claude --help` to confirm the flag is `--allowedTools` (or `--allowed-tools`)
- The web search tool name might be `WebSearch`, `web_search`, or `WebFetch` — verify from Claude CLI documentation
- DO NOT enable any file or command execution tools (Bash, Edit, Write, Read, etc.)
- If unsure about the flag, test with `claude -p --allowedTools "WebSearch" --output-format text` before implementing

### CRITICAL: Agent Prompt Design — No Formal Mode Engine

The PRD explicitly states scenarios require NO formal mode switching. The user says something like "let's practice interview questions" and Claude adapts its behavior. Implementation is entirely in the system prompt.

**Deferred feedback pattern:**
- Default: real-time corrections (current 4.1 behavior)
- User says "switch to deferred" or "save corrections for later" → Claude acknowledges, stops inline corrections
- Claude internally tracks errors encountered (in its conversation context, NOT in files)
- When user says "give me my feedback" or "let's wrap up" → Claude presents collected corrections as a summary
- User says "switch back to real-time" → resume inline corrections

**Scenario transitions:**
- No formal "mode" concept — Claude naturally adapts based on user's request
- Transition acknowledgment should be brief (1 sentence) then immediately start the scenario
- Multiple scenarios can blend (e.g., grammar drills within an interview simulation)

### CRITICAL: Voice Output Format Compliance

All new prompt sections MUST follow the Voice Output Format rules from story 4-1:
- No markdown formatting in the prompt's EXAMPLE responses (no bold, italic, headers)
- Example responses should be 2-4 sentences
- Natural spoken language only

However, the prompt itself (the system prompt instructions TO the LLM) CAN use markdown for structure since it's never spoken aloud.

### CRITICAL: LLM-Backend Agnosticism (NFR9) Still Applies

All new sections in `language_trainer.agent.md` must:
- NOT reference "Claude", "Anthropic", or any specific LLM
- NOT reference Claude-specific tools like "WebSearch" by name in the agent file
- Use generic language: "search the web" not "use the WebSearch tool"
- The agent file must remain portable across LLM backends

### CRITICAL: Existing Prompt Structure to Preserve

The current `language_trainer.agent.md` (118 lines from story 4-1) has these sections:
1. Header + core description (line 1-3)
2. Voice Output Format (lines 5-13)
3. Core Persona (lines 15-22)
4. CEFR-Aware Methodology (lines 24-49)
5. Real-Time Correction Approach (lines 51-93)
6. Conversation Flow Guidelines (lines 95-101)
7. Session Sustainability (lines 103-111)
8. Boundaries (lines 113-118)

**New sections should be inserted AFTER "Real-Time Correction Approach" (line 93) and BEFORE "Conversation Flow Guidelines" (line 95).** This keeps correction-related content grouped and flows logically: corrections → feedback modes → scenarios → conversation flow → sustainability.

Recommended insertion order:
- After line 93: new "## Feedback Modes" section
- After feedback modes: new "## Scenario Handling" section
- Then existing "Conversation Flow Guidelines", "Session Sustainability", "Boundaries"

### Previous Story Intelligence (from Story 4-1)

- **Story 4-1 was entirely a prompt story** — expanded agent file from 1 line to 118 lines
- **Code review fixed**: added Voice Output Format section, removed bold from examples, added CEFR fallback for A1/C1+
- **Key pattern**: inline `#[cfg(test)]` modules, `anyhow::Result`, `make check` for verification
- **Agent file is passed inline** via `cmd.args(["--system-prompt", &system_prompt])` — no file path, direct content
- **Length limit**: system prompt is a CLI argument; 118 lines (6,729 bytes) is well within shell limits (128KB-2MB). Adding ~80-100 lines is safe.
- **Correction techniques established**: conversational recast (60-70%), brief explicit (20-30%), positive reinforcement — new sections should reference these, not redefine them
- **TTS awareness**: all LLM output goes through TTS — responses must be plain spoken language
- **Task 4 (manual E2E) was deferred** in 4-1 due to TTS crash (sherpa-onnx version mismatch). Same limitation applies to 4-2.

### Git Intelligence (Recent Commits)

```
9c9ba85 Expand language coaching agent prompt with CEFR methodology and TTS awareness
103df08 Add session control, error recovery, and voice loop integration
f9927c4 Replace SSH client with TCP connection and add TTS audio playback
5c25956 Add server dual listeners and message routing (TCP + Unix socket)
69a31c9 Integrate TTS engine with sherpa-rs Kokoro support
c2ad8e4 Add orchestrator Claude CLI bridge with LlmBackend trait
```

Patterns from recent commits:
- One commit per story (focused changes)
- Descriptive commit messages without "Claude" references
- `make check` used consistently for verification

### Testing Strategy

1. **`make check`** — all existing tests pass (no regressions)
2. **NFR9 verification** — `grep -i "claude\|anthropic" agent/language_trainer.agent.md` returns 0 matches
3. **Prompt length check** — verify file stays under ~15KB (well within CLI limits)
4. **Manual E2E** — deferred (same TTS limitation as 4-1; test plan documented in Task 4)

### Project Structure Notes

- Alignment with unified project structure: `agent/` for agent definitions, `orchestrator/src/` for CLI bridge code
- No new files created — only modifications to existing files
- No new dependencies needed

### References

- [Source: epics.md#Epic 4, Story 4.2] — Acceptance criteria, FR26, FR28
- [Source: prd.md#FR12] — Web search without user approval
- [Source: prd.md#FR26] — Deferred feedback mode
- [Source: prd.md#FR28] — Scenario types (free conversation, grammar drills, interview simulation, topic discussion with web search, level assessment)
- [Source: prd.md#Design Philosophy] — "Adaptive scenarios driven entirely by Claude's contextual intelligence — no formal scenario engine required"
- [Source: architecture.md#Claude CLI Integration] — `--system-prompt` flag, `--continue` for session continuity
- [Source: architecture.md#Agent Definition Decoupling] — NFR9, LLM-backend-agnostic requirement
- [Source: 4-1-core-language-coaching-persona-and-real-time-feedback.md] — Previous story learnings, agent prompt structure, Voice Output Format

## Dev Agent Record

### Agent Model Used

Claude Opus 4.6

### Debug Log References

None — this story involves primarily system prompt expansion and one code constant change.

### Completion Notes List

1. **Task 1 — Feedback Modes section added to agent prompt.** New "## Feedback Modes" section (lines 95-113) covers: deferred feedback mode definition, vocal trigger phrases for switching ("save corrections for later", "switch to deferred feedback"), acknowledgment format (brief 1-sentence), behavior change (stop inline corrections, mentally track errors), summary presentation (3-5 key patterns, constructive framing), and switching back to real-time.

2. **Task 2 — Scenario Handling section added to agent prompt.** New "## Scenario Handling" section (lines 115-147) covers all 5 FR28 scenario types:
   - Free conversation (default, open-ended dialogue)
   - Grammar drills (brisk pace, sentence exercises, conversational tone)
   - Interview simulation (professional interviewer role, feedback on language + content)
   - Topic discussion (web search for current info, vocabulary introduction, debate for advanced learners)
   - Level assessment (structured 5-10 exchange assessment, spoken CEFR estimate, positive framing)
   Seamless transitions via vocal request, no formal mode switching per PRD design philosophy.

3. **Task 3 — Web search enabled in Claude CLI.** Changed `ALLOWED_TOOLS` constant from `""` (all tools disabled) to `"WebSearch"` in `orchestrator/src/claude.rs`. The `--tools` flag (confirmed via `claude --help`) uses PascalCase tool names. Added unit test `allowed_tools_enables_web_search` verifying the constant is non-empty and contains "WebSearch". `make check` passes: 85 tests (21 client + 34 common + 14 orchestrator + 16 server), +1 new test.

4. **Task 4 — Manual E2E test deferred.** Same TTS infrastructure limitation as story 4-1 (sherpa-onnx v1.12.9 crashes during synthesis due to version mismatch with kokoro-multi-lang-v1_0 model). Test plan documented:
   - Start full stack: server, orchestrator (--agent agent/language_trainer.agent.md), client
   - Test deferred feedback: say "switch to deferred feedback", make errors, say "give me my feedback", verify summary
   - Test scenarios: request grammar drill, interview simulation, topic discussion — verify smooth transitions
   - Test web search: request "let's discuss recent AI news" — verify web search triggers and enriches conversation

5. **NFR9 verified**: `grep -i "claude|anthropic|AI assistant"` → 0 matches in agent file. Web search referenced as "search the web" (generic), not "use WebSearch tool".

6. **Agent file stats**: 171 lines, 10,225 bytes (expanded from 118 lines/6,729 bytes). Well within CLI argument limits.

### File List

- `agent/language_trainer.agent.md` (MODIFIED) — Added Feedback Modes section (deferred/real-time switching) and Scenario Handling section (5 scenario types)
- `orchestrator/src/claude.rs` (MODIFIED) — Added `ALLOWED_TOOLS` constant set to `"WebSearch"`, changed `--tools ""` to `--tools ALLOWED_TOOLS`, added unit test

### Code Review Record

**Reviewer:** Claude Opus 4.6 (adversarial review)
**Date:** 2026-02-21
**Findings:** 0 HIGH, 3 MEDIUM, 3 LOW (6 total)
**Resolution:** All 6 issues fixed automatically

**Issues fixed:**
- M1: Added length exception for feedback summaries and level assessments in Voice Output Format (line 12)
- M2: Changed "sentence with a gap" to "sentence with an intentional error" for spoken grammar drills (line 128)
- M3: Added web search fallback guidance: "If search results are unavailable, continue using general knowledge" (line 140)
- L1: Added scenario exit guidance: "smoothly return to free conversation" when user wants to change activities (line 118)
- L2: Added cross-reference in Level Assessment: "This is distinct from the automatic level detection" (line 146)
- L3: Noted pre-existing uncommitted TTS lang fix changes in `server/src/main.rs` and `server/src/tts.rs` (from previous session, not story 4-2)

**Post-fix verification:** `make check` → 85 tests pass, NFR9 grep → 0 matches
