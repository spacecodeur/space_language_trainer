#!/usr/bin/env bash
set -euo pipefail

# Allow running from within a Claude Code session
unset CLAUDECODE 2>/dev/null || true

# =============================================================================
# Phase 0 Spike: Validate Claude CLI --continue session continuity
# =============================================================================
# Tests:
#   1. Context preservation over 20+ sequential turns with --continue
#   2. --system-prompt persistence across turns
#   3. stdout cleanliness (no stderr pollution)
#   4. Fork/exec overhead per invocation
# =============================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESULTS_DIR="$SCRIPT_DIR/results"
WORK_DIR=$(mktemp -d "/tmp/spike-claude-cli-XXXXXX")
trap 'rm -rf "$WORK_DIR"' EXIT

# Minimal English tutor persona
SYSTEM_PROMPT="You are a patient, encouraging English language tutor. You help the user practice English conversation. Correct grammar mistakes inline but do not break the conversation flow. Keep responses concise (2-3 sentences max). When asked to recall or list topics, be precise and complete."

# Clean up previous results
rm -rf "$RESULTS_DIR"
mkdir -p "$RESULTS_DIR"

info()  { printf '\033[1;34m[INFO]\033[0m  %s\n' "$*"; }
fail()  { printf '\033[1;31m[FAIL]\033[0m  %s\n' "$*"; }

# ---------------------------------------------------------------------------
# 22 prompts: introduce numbered topics, with recall checks at turns 6, 11,
# 16, and a final full-recall at turn 22.
# ---------------------------------------------------------------------------
PROMPTS=(
  # Turn 1-5: Introduce topics
  "Hello! Let's practice English. I want to talk about topic 1: my favorite hobby is cooking Italian food."
  "Great! Now topic 2: I went to New Zealand last summer and visited Queenstown."
  "Topic 3: I have been learning to play the piano for six months now."
  "Topic 4: My sister work as a doctor in a big hospital in Lyon."
  "Topic 5: I am planning to adopt a cat next month, probably a ginger one."

  # Turn 6: RECALL CHECK (short-term)
  "Before we continue, can you briefly list all 5 topics we have discussed so far? Number them 1 through 5."

  # Turn 7-10: More topics
  "Topic 6: I thinked about changing my career to become a software developer."
  "Topic 7: My favorite movie is Inception by Christopher Nolan."
  "Topic 8: I have a garden where I grow tomatoes, basil, and strawberries."
  "Topic 9: Last weekend I runned a half-marathon in 1 hour 45 minutes."

  # Turn 11: RECALL CHECK (medium-term)
  "Can you list all 9 topics we have discussed so far? Please number them 1 through 9."

  # Turn 12-15: More topics
  "Topic 10: I want to improve my English before moving to New Zealand in May 2026."
  "Topic 11: I enjoy reading science fiction books, especially by Isaac Asimov."
  "Topic 12: My friends and me went to a jazz concert last Friday."
  "Topic 13: I have been try to reduce my screen time to less than 2 hours per day."

  # Turn 16: RECALL CHECK (medium-long)
  "Please list all 13 topics we have discussed. Number them 1 through 13."

  # Turn 17-21: Final topics
  "Topic 14: I learned to make sushi at a cooking class last month."
  "Topic 15: My dream is to visit Japan and climb Mount Fuji someday."
  "Topic 16: I play football every Wednesday evening with colleagues."
  "Topic 17: I have a collection of vinyl records, mostly 1970s rock music."
  "Topic 18: Next year I want to run my first full marathon."

  # Turn 22: FINAL FULL RECALL
  "This is our last exchange. Please list ALL 18 topics we discussed in this conversation, numbered 1 through 18. Also, have you been acting as an English tutor throughout? Did you correct my grammar mistakes?"
)

TURN_COUNT=${#PROMPTS[@]}
info "Starting Claude CLI spike test with $TURN_COUNT turns"
info "Working directory: $WORK_DIR"
info "Results directory: $RESULTS_DIR"
echo

# Change to isolated temp directory so --continue doesn't pick up
# conversations from the project directory
cd "$WORK_DIR"

# ---------------------------------------------------------------------------
# Execute each turn
# ---------------------------------------------------------------------------
total_overhead_ms=0
overhead_samples=0
success_count=0

for i in $(seq 0 $((TURN_COUNT - 1))); do
  turn_num=$((i + 1))
  turn_label=$(printf "turn-%02d" "$turn_num")
  prompt="${PROMPTS[$i]}"

  info "Turn $turn_num/$TURN_COUNT: ${prompt:0:80}..."

  # Build command (prompt via stdin because --tools is variadic and
  # would consume a positional prompt argument)
  if [ "$turn_num" -eq 1 ]; then
    # First turn: establish session with system prompt
    cmd=(claude -p --system-prompt "$SYSTEM_PROMPT" --output-format text --tools "")
  else
    # Subsequent turns: continue session
    cmd=(claude -p --continue --output-format text --tools "")
  fi

  # Execute with timing (pipe prompt through stdin)
  start_ns=$(date +%s%N)

  echo "$prompt" | "${cmd[@]}" \
    > "$RESULTS_DIR/$turn_label.stdout" \
    2> "$RESULTS_DIR/$turn_label.stderr" \
    || true

  end_ns=$(date +%s%N)
  elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))

  # Save timing
  echo "$elapsed_ms" > "$RESULTS_DIR/$turn_label.time"

  # Display result summary
  stdout_size=$(wc -c < "$RESULTS_DIR/$turn_label.stdout")
  stderr_size=$(wc -c < "$RESULTS_DIR/$turn_label.stderr")

  if [ "$stdout_size" -gt 0 ]; then
    printf "  Response (%d ms, %d bytes stdout, %d bytes stderr)\n" "$elapsed_ms" "$stdout_size" "$stderr_size"
    # Show first 200 chars of response
    head -c 200 "$RESULTS_DIR/$turn_label.stdout"
    echo
    echo "---"
    success_count=$((success_count + 1))
  else
    fail "  No stdout output! (${elapsed_ms}ms, stderr: ${stderr_size} bytes)"
  fi

  # Track overhead for short prompts (approximate fork/exec cost)
  total_overhead_ms=$((total_overhead_ms + elapsed_ms))
  overhead_samples=$((overhead_samples + 1))

  # Small delay to avoid rate limiting
  sleep 1
done

echo
info "All $TURN_COUNT turns completed."
echo

# ---------------------------------------------------------------------------
# Generate summary
# ---------------------------------------------------------------------------
info "Generating summary..."

{
  echo "============================================"
  echo "Phase 0 Spike: Claude CLI Session Continuity"
  echo "Date: $(date -Iseconds)"
  echo "Turns: $TURN_COUNT"
  echo "Working directory: $WORK_DIR"
  echo "============================================"
  echo

  # Timing summary
  echo "## Timing (ms per turn)"
  echo
  for i in $(seq 0 $((TURN_COUNT - 1))); do
    turn_num=$((i + 1))
    turn_label=$(printf "turn-%02d" "$turn_num")
    time_ms=$(cat "$RESULTS_DIR/$turn_label.time" 2>/dev/null || echo "N/A")
    printf "  Turn %2d: %s ms\n" "$turn_num" "$time_ms"
  done
  echo
  avg_ms=$((total_overhead_ms / overhead_samples))
  echo "  Average: ${avg_ms} ms"
  echo

  # Stderr check
  echo "## Stderr Output"
  echo
  has_stderr=false
  for i in $(seq 0 $((TURN_COUNT - 1))); do
    turn_num=$((i + 1))
    turn_label=$(printf "turn-%02d" "$turn_num")
    stderr_size=$(wc -c < "$RESULTS_DIR/$turn_label.stderr" 2>/dev/null || echo "0")
    if [ "$stderr_size" -gt 0 ]; then
      has_stderr=true
      printf "  Turn %2d: %s bytes stderr\n" "$turn_num" "$stderr_size"
    fi
  done
  if [ "$has_stderr" = false ]; then
    echo "  No stderr output on any turn. CLEAN."
  fi
  echo

  # Recall checks
  echo "## Recall Check Results"
  echo
  for recall_turn in 6 11 16 22; do
    turn_label=$(printf "turn-%02d" "$recall_turn")
    echo "  Turn $recall_turn (recall check):"
    if [ -f "$RESULTS_DIR/$turn_label.stdout" ]; then
      echo "  --- Response ---"
      cat "$RESULTS_DIR/$turn_label.stdout"
      echo "  --- End ---"
    else
      echo "  NO OUTPUT"
    fi
    echo
  done

  echo "## Manual Verification Required"
  echo
  echo "  Review recall turns (6, 11, 16, 22) to verify:"
  echo "  1. Context retention: Does Claude recall all previous topics?"
  echo "  2. System prompt: Does Claude maintain English tutor persona?"
  echo "  3. Grammar corrections: Did Claude correct deliberate errors?"
  echo "     - Turn 4: 'My sister work' -> 'My sister works'"
  echo "     - Turn 7: 'I thinked' -> 'I thought'"
  echo "     - Turn 10: 'I runned' -> 'I ran'"
  echo "     - Turn 14: 'My friends and me' -> 'My friends and I'"
  echo "     - Turn 15: 'I have been try' -> 'I have been trying'"
  echo

} > "$RESULTS_DIR/summary.txt"

cat "$RESULTS_DIR/summary.txt"

info "Results saved to $RESULTS_DIR/"
info "Successful turns: $success_count/$TURN_COUNT"

if [ "$success_count" -eq 0 ]; then
  fail "All turns failed — no stdout output on any turn."
  exit 1
fi
