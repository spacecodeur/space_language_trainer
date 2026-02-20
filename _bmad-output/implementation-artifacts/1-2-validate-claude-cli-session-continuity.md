# Story 1.2: Validate Claude CLI Session Continuity (Phase 0 Spike)

Status: done

<!-- Note: Validation is optional. Run validate-create-story for quality check before dev-story. -->

## Story

As a **developer**,
I want to validate that Claude CLI `--continue` preserves conversation context over 20+ sequential turns,
so that I can confirm the core technical assumption before investing in Rust development.

## Acceptance Criteria

1. **Given** Claude CLI is installed and functional
   **When** the developer runs a scripted test sending 20+ sequential prompts with `claude -p --continue`
   **Then** Claude's responses demonstrate context awareness of the full conversation history

2. **Given** the scripted test
   **When** `--system-prompt` is combined with `--continue`
   **Then** the system prompt is respected across all turns (persona maintained throughout)

3. **Given** the scripted test with `-p` (print mode)
   **When** capturing stdout
   **Then** stdout contains only Claude's response text (no stderr pollution, no tool-use artifacts)

4. **Given** the scripted test
   **When** measuring fork/exec overhead per turn
   **Then** overhead is documented (target: <500ms per invocation, excluding Claude API response time)

5. **Given** test results
   **When** the developer evaluates all criteria
   **Then** a go/no-go decision is documented in `docs/spike-claude-cli.md`

6. **Given** the spike fails on any critical criterion
   **When** the go/no-go decision is "no-go"
   **Then** the project is reassessed (hard gate — this blocks all further development)

## Tasks / Subtasks

- [x] Task 1: Create spike test script (AC: #1, #2, #3)
  - [x] 1.1: Create `spike/claude-cli-test.sh` — a bash script that sends 20+ sequential prompts to Claude CLI
  - [x] 1.2: Each prompt must reference earlier conversation context (e.g., "What was the 3rd topic we discussed?")
  - [x] 1.3: Use `--system-prompt` with a minimal English tutor persona (see Dev Notes for the workaround)
  - [x] 1.4: Capture stdout and stderr separately for each turn
  - [x] 1.5: Measure wall-clock time per invocation (total and excluding API response)

- [x] Task 2: Create context verification prompts (AC: #1)
  - [x] 2.1: Design 20+ prompts that build on each other (numbered topics introduced sequentially)
  - [x] 2.2: Include "recall" prompts every 5 turns that ask Claude to summarize prior conversation
  - [x] 2.3: Include a final prompt asking Claude to list ALL topics discussed (tests full context retention)

- [x] Task 3: Run the spike and capture results (AC: #1, #2, #3, #4)
  - [x] 3.1: Run `spike/claude-cli-test.sh` and capture all output to `spike/results/`
  - [x] 3.2: Verify context awareness at each recall point
  - [x] 3.3: Verify system prompt persona is maintained across all turns
  - [x] 3.4: Verify stdout is clean (no stderr leakage, no tool artifacts)
  - [x] 3.5: Document per-turn timing (fork/exec overhead vs API response time)

- [x] Task 4: Document go/no-go decision (AC: #5, #6)
  - [x] 4.1: Create `docs/spike-claude-cli.md` with findings:
    - Context retention results (pass/fail at each recall point)
    - System prompt persistence (pass/fail)
    - stdout cleanliness (pass/fail)
    - Fork/exec overhead measurements
    - Go/no-go decision with rationale
  - [x] 4.2: If go: document the exact invocation pattern to use in the orchestrator
  - [ ] 4.3: If no-go: document what failed and alternatives to explore (N/A — go decision)

- [x] Task 5: Verify build still passes (AC: N/A — hygiene)
  - [x] 5.1: Run `make check` — must still pass (no Rust code changes expected)

## Dev Notes

### CRITICAL: `--system-prompt-file` Does NOT Exist

The architecture document references `--system-prompt-file` as a Claude CLI flag. **This flag does not exist.** The actual available flag is:

```
--system-prompt <prompt>    System prompt to use for the session
```

This takes an **inline string**, not a file path. Workarounds:

1. **Shell substitution:** `claude -p --system-prompt "$(cat language_trainer.agent.md)" --continue "prompt"`
2. **Heredoc or variable:** Load file content into a variable, pass as argument

The spike must validate which approach works reliably, especially with `--continue`. If the system prompt is too long for command-line argument limits, document the constraint.

### Exact Claude CLI Invocation Pattern

Based on `claude --help`, the expected pattern is:

```bash
# First turn (establishes session):
claude -p --system-prompt "You are an English tutor..." "Hello, let's practice English"

# Subsequent turns (continue session):
claude -p --continue "What did we talk about?"
```

**Key flags:**
- `-p, --print` — Non-interactive mode, prints response and exits
- `-c, --continue` — Continue the most recent conversation in the current directory
- `--system-prompt <prompt>` — Inline system prompt string
- `--output-format text` — Default, plain text output (verify this is clean)
- `--resume <session-id>` — Alternative to `--continue` using explicit session ID
- `--tools ""` — Disable all tools (may reduce overhead and prevent tool-use artifacts in stdout)

### Session Management Considerations

`--continue` continues "the most recent conversation **in the current directory**". This means:
- The script must run from a consistent directory
- Multiple concurrent sessions could conflict (investigate `--session-id` and `--resume` as alternatives)
- The `--resume <session-id>` flag may be more robust for production use

### What "Context Awareness" Means for Validation

The test should verify:
1. **Short-term recall** (1-3 turns back): Claude references the immediately preceding exchange
2. **Medium-term recall** (5-10 turns back): Claude recalls specific topics or details from earlier
3. **Long-term recall** (15-20+ turns back): Claude can list or summarize the full conversation history
4. **System prompt persistence**: The English tutor persona remains active (doesn't drop after N turns)

### Measuring Fork/Exec Overhead

Use `time` or bash `SECONDS`/`date` arithmetic to measure:
- **Total wall time** per invocation (includes API response time)
- **Fork/exec overhead** is harder to isolate — approximate by comparing total time vs API response time
- A simple approach: send a very short prompt like "say ok" and measure total time — the response time for a 2-token response is negligible, so wall time ≈ fork/exec + network round-trip + minimal processing

Target: <500ms overhead per invocation. If significantly higher, document and assess impact.

### Minimal English Tutor Persona for Testing

```
You are a patient, encouraging English language tutor. You help the user practice English conversation. Correct grammar mistakes inline but don't break the conversation flow. Keep responses concise (2-3 sentences max).
```

### Script Output Structure

```
spike/
├── claude-cli-test.sh      # Test script
├── results/
│   ├── turn-01.stdout      # stdout per turn
│   ├── turn-01.stderr      # stderr per turn
│   ├── turn-01.time        # timing data per turn
│   ├── ...
│   └── summary.txt         # Pass/fail summary
docs/
└── spike-claude-cli.md     # Go/no-go decision document
```

### This is a Spike, Not a Rust Story

This story does **not** modify any Rust code. It is a pure shell-script validation exercise. The output is documentation (`docs/spike-claude-cli.md`) and a test script (`spike/`). No changes to `common/`, `client/`, `server/`, or `orchestrator/`.

### Previous Story Intelligence (from Story 1-1)

- Workspace is set up with 4 crates, `make check` passes (33 tests)
- Package naming: `space_lt_*` (underscore)
- Makefile exists — always use `make check` not raw cargo commands
- `.gitignore` excludes `/target`

### References

- [Source: architecture.md#Claude CLI Integration] — Invocation pattern, Phase 0 spike requirements
- [Source: architecture.md#Decision Impact Analysis] — Phase 0 spike is a hard gate
- [Source: architecture.md#LLM Backend Abstraction] — LlmBackend trait that will consume spike findings
- [Source: epics.md#Story 1.2] — Acceptance criteria
- [Source: epics.md#Epic 1] — Phase 0 spike is go/no-go gate for entire project

## Dev Agent Record

### Agent Model Used

Claude Opus 4.6

### Debug Log References

None

### Completion Notes List

- Created `spike/claude-cli-test.sh` with 22-turn conversation test (18 topics + 4 recall checkpoints)
- Discovered and fixed nested session issue: must `unset CLAUDECODE` to run Claude CLI from within a Claude Code session
- Discovered `--tools` is variadic: prompt must be piped through stdin (not positional arg) when using `--tools ""`
- Fixed CWD isolation: script `cd`s to temp directory so `--continue` doesn't pick up project conversations
- All 22 turns completed successfully: 100% recall accuracy, all 5 grammar corrections detected, clean stdout
- Fork/exec overhead ~1–1.4s (Node.js startup dominated), acceptable for voice use case
- Created `docs/spike-claude-cli.md` with GO decision and documented invocation pattern
- `make check` passes (33 tests, fmt clean, clippy clean — no Rust code changes in this story)

**Code review fixes (adversarial review):**
- Added `spike/results/` and `/.claude/` to `.gitignore` (HIGH: generated files + user config would be committed)
- Added `trap 'rm -rf "$WORK_DIR"' EXIT` for temp directory cleanup (MEDIUM: orphaned session data)
- Removed dead `pass()` and `warn()` functions (LOW: dead code)
- Added `success_count` tracking and `exit 1` on total failure (LOW: script always exited 0)

### File List

- `spike/claude-cli-test.sh` — Phase 0 spike test script (new, review fixes applied)
- `spike/results/` — generated test outputs (gitignored, not committed)
- `docs/spike-claude-cli.md` — Go/no-go decision document (new)
- `.gitignore` — updated (added `/.claude/`, `spike/results/`)
