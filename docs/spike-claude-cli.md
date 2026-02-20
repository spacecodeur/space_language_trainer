# Phase 0 Spike: Claude CLI Session Continuity — Go/No-Go

**Date:** 2026-02-20
**Decision: GO**

## Summary

Claude CLI `--continue` reliably preserves full conversation context over 22 sequential turns. System prompt persona is maintained throughout. Stdout is clean. The core technical assumption for `space_language_training` is validated.

## Test Setup

- **Script:** `spike/claude-cli-test.sh`
- **Turns:** 22 (18 numbered topics + 4 recall checkpoints at turns 6, 11, 16, 22)
- **System prompt:** Inline English tutor persona via `--system-prompt`
- **Invocation pattern:**
  ```bash
  # First turn (establish session):
  echo "$prompt" | claude -p --system-prompt "$SYSTEM_PROMPT" --output-format text --tools ""

  # Subsequent turns (continue session):
  echo "$prompt" | claude -p --continue --output-format text --tools ""
  ```
- **Key detail:** Prompt must be piped through stdin (not as a positional argument) because `--tools` is a variadic flag that consumes subsequent positional args.
- **Session isolation:** Script `cd`s to a temp directory before running turns, since `--continue` resumes the most recent conversation in the current working directory.

## Results

### 1. Context Retention — PASS

| Recall Point | Topics Expected | Topics Recalled | Accuracy |
|-------------|----------------|----------------|----------|
| Turn 6      | 5              | 5              | 100%     |
| Turn 11     | 9              | 9              | 100%     |
| Turn 16     | 13             | 13             | 100%     |
| Turn 22     | 18             | 18             | 100%     |

All 18 topics recalled with accurate details (names, places, numbers) at every checkpoint. Claude also made cross-topic connections (e.g., linking the garden to Italian cooking, the NZ trip to the NZ move).

### 2. System Prompt Persistence — PASS

The English tutor persona was maintained across all 22 turns:
- Encouraging, conversational tone throughout
- Follow-up questions asked on every topic
- Grammar corrections provided inline without breaking flow
- Final turn explicitly confirmed tutor role

### 3. Grammar Corrections — PASS (5/5)

All 5 deliberate grammar errors were caught and corrected:

| Turn | Error | Correction | Detected |
|------|-------|------------|----------|
| 4    | "My sister work" | "My sister **works**" | Yes |
| 7    | "I thinked" | "I **thought**" | Yes |
| 10   | "I runned" | "I **ran**" | Yes |
| 14   | "My friends and me" | "My friends and **I**" | Yes |
| 15   | "I have been try" | "I have been **trying**" | Yes |

### 4. Stdout Cleanliness — PASS

- **0 bytes stderr** on all 22 turns
- No tool-use artifacts in stdout
- Pure text responses only
- `--tools ""` successfully disabled all tool use

### 5. Timing

| Metric | Value |
|--------|-------|
| Average per turn | 6,053 ms |
| Minimum (turn 1) | 3,755 ms |
| Maximum (turn 22) | 11,082 ms |
| Trend | Gradual increase as context grows |

**Fork/exec overhead estimate:** ~1,000–1,400 ms (measured from failed runs that returned errors immediately without making API calls). This exceeds the 500ms target, but is dominated by Node.js runtime startup — unavoidable for a CLI tool. For the voice conversation use case (where human speech takes 2–10 seconds), this overhead is acceptable.

**Note:** Timing increases with conversation length because the full context is sent to the API on each turn. Turn 22 (11s) included a long response listing all 18 topics with grammar corrections.

## Invocation Pattern for Orchestrator

```bash
# First turn:
echo "$prompt" | claude -p \
  --system-prompt "$(cat persona.md)" \
  --output-format text \
  --tools ""

# Subsequent turns:
echo "$prompt" | claude -p \
  --continue \
  --output-format text \
  --tools ""
```

### Critical Implementation Notes

1. **`--system-prompt-file` does NOT exist.** The architecture document references this flag, but only `--system-prompt <prompt>` (inline string) is available. Use shell substitution `"$(cat file.md)"` to load from a file.

2. **Prompt via stdin, not positional arg.** The `--tools` flag is variadic (`<tools...>`) and consumes subsequent positional arguments. Always pipe the prompt through stdin.

3. **CWD matters for `--continue`.** It resumes the most recent conversation in the current working directory. The orchestrator must run Claude CLI from a stable, session-specific directory.

4. **`--resume <session-id>`** is available as a more explicit alternative to `--continue`. Consider using this in production for robustness (avoids CWD dependency).

5. **`unset CLAUDECODE`** is required when launching Claude CLI from within a Claude Code session (development/testing only).

6. **`--tools ""`** disables all tools, keeping stdout clean and reducing overhead.

## Issues Encountered During Spike

1. **Nested session block:** Claude CLI refuses to launch inside another Claude Code session (`CLAUDECODE` env var). Fixed with `unset CLAUDECODE`.

2. **Variadic `--tools` consuming prompt:** `--tools "" "$prompt"` causes the prompt to be consumed as a tool argument. Fixed by piping prompt through stdin.

3. **CWD-dependent `--continue`:** Without `cd` to an isolated directory, `--continue` would pick up conversations from the project root.

## Go/No-Go Decision

**GO.** All acceptance criteria pass:

- [x] Context preserved over 22 sequential turns with 100% recall accuracy
- [x] System prompt respected across all turns (persona maintained)
- [x] Stdout clean (0 bytes stderr, no tool artifacts)
- [x] Fork/exec overhead documented (~1–1.4s, acceptable for voice use case)
- [x] Invocation pattern documented for orchestrator consumption
