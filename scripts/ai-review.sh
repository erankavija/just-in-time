#!/usr/bin/env bash
set -euo pipefail

# AI-powered review gate checker for jit.
#
# Pipes a structured review prompt (issue context + gate instructions) into an
# AI agent CLI and parses a VERDICT: PASS / VERDICT: FAIL from its output.
#
# Contract:
#   exit 0  — gate passed
#   exit 1  — gate failed (or verdict could not be parsed)
#   stdout  — full review text, captured by jit in gate run results
#   stderr  — errors, shown by jit on failure
#
# Requires:
#   JIT_CONTEXT_FILE  — set automatically by jit for --pass-context gates
#   REVIEWER_AGENT    — command that reads a prompt from stdin and writes a
#                       review to stdout. Evaluated as a shell command.
#
# Example REVIEWER_AGENT values:
#   codex review -
#   copilot -s --model claude-haiku-4.5
#   claude --model haiku -p -
#   cat                                    # dry-run (echoes the prompt)
#
# Setup:
#   1. Copy this script into your repo (e.g. scripts/ai-review.sh)
#   2. chmod +x scripts/ai-review.sh
#   3. Define the gate:
#        jit gate define ai-review \
#          --title "AI Code Review" \
#          --description "AI-powered code review" \
#          --mode auto --stage postcheck \
#          --pass-context \
#          --prompt "Review the implementation for correctness and style." \
#          --checker-command "./scripts/ai-review.sh" \
#          --env REVIEWER_AGENT="codex review -" \
#          --timeout 120
#   4. Run: jit gate check <issue> ai-review

if [ -z "${JIT_CONTEXT_FILE:-}" ]; then
  echo "ERROR: JIT_CONTEXT_FILE not set. This gate requires --pass-context." >&2
  exit 1
fi

if [ ! -f "$JIT_CONTEXT_FILE" ]; then
  echo "ERROR: Context file not found: $JIT_CONTEXT_FILE" >&2
  exit 1
fi

if [ -z "${REVIEWER_AGENT:-}" ]; then
  echo "ERROR: REVIEWER_AGENT not set." >&2
  echo "  Set it to a command that reads a prompt from stdin and writes to stdout." >&2
  echo "  Example: REVIEWER_AGENT='codex review -'" >&2
  exit 1
fi

PROMPT=$(jq -r '.prompt // empty' "$JIT_CONTEXT_FILE")

if [ -z "${PROMPT:-}" ]; then
  echo "ERROR: No prompt defined for this gate. Set --prompt or --prompt-file when defining the gate." >&2
  exit 1
fi

# Extract structured fields from context so the prompt leads the input.
CONTEXT_JSON=$(jq -c 'del(.prompt)' "$JIT_CONTEXT_FILE")

# Capture agent stderr to a temp file so we can surface it on errors.
AGENT_STDERR=$(mktemp)
trap 'rm -f "$AGENT_STDERR"' EXIT

# Feed the agent: prompt first, then context data, then verdict instruction.
REVIEW_OUTPUT=$(cat <<EOF | eval "$REVIEWER_AGENT" 2>"$AGENT_STDERR"
${PROMPT}

## Context

\`\`\`json
${CONTEXT_JSON}
\`\`\`

Before the verdict line, output a numbered list of every finding across all categories, followed by a single line stating the total count (e.g., "Total findings: N"). All findings must appear in this single enumeration — none may be withheld for a later round.

You MUST end your response with exactly one of these lines:
VERDICT: PASS
VERDICT: FAIL
No text may follow the verdict line.
EOF
) || true

# A reviewer agent's stderr can be enormous -- it may echo its entire prompt and
# context back (observed: thousands of lines per run). jit stores this verbatim
# in the gate run result, and a --pass-context gate feeds prior runs back into
# the next review round, so an unbounded dump both bloats .jit/gate-runs/ and
# drowns the next reviewer. The one durably useful line is the session id, which
# points at the agent's full transcript -- resume that to see everything. So we
# keep only through the session id plus a couple lines, and drop the rest.
#
# Tunables:
#   AGENT_STDERR_ID_PATTERN -- grep -i pattern locating the transcript pointer
#   AGENT_STDERR_AFTER_ID   -- extra lines kept after the matched line
#   AGENT_STDERR_HEAD_LINES -- fallback head when no pointer line is found
AGENT_STDERR_ID_PATTERN="${AGENT_STDERR_ID_PATTERN:-session id}"
AGENT_STDERR_AFTER_ID="${AGENT_STDERR_AFTER_ID:-2}"
AGENT_STDERR_HEAD_LINES="${AGENT_STDERR_HEAD_LINES:-12}"

# Surface a tight slice of the agent's stderr: through the session id (+ a couple
# lines), or a small head if no session id is present. Never called on the PASS
# path -- a passing review's stderr is pure noise.
show_agent_stderr() {
  if [ ! -s "$AGENT_STDERR" ]; then
    return
  fi
  total=$(wc -l <"$AGENT_STDERR")
  id_line=$(grep -in "$AGENT_STDERR_ID_PATTERN" "$AGENT_STDERR" | head -1 | cut -d: -f1 || true)
  if [ -n "$id_line" ]; then
    keep=$((id_line + AGENT_STDERR_AFTER_ID))
  else
    keep="$AGENT_STDERR_HEAD_LINES"
  fi
  echo "--- agent stderr (first ${keep} of ${total} lines; the session id below points to the full agent transcript) ---" >&2
  head -n "$keep" "$AGENT_STDERR" >&2
  if [ "$total" -gt "$keep" ]; then
    echo "--- $((total - keep)) more lines omitted (resume the session for the rest) ---" >&2
  fi
}

if [ -z "$REVIEW_OUTPUT" ]; then
  echo "---" >&2
  echo "ERROR: Agent produced no output." >&2
  show_agent_stderr
  exit 1
fi

echo "$REVIEW_OUTPUT"

# Extract verdict from the last non-blank line (portable — works on BSD and GNU).
LAST_LINE=$(echo "$REVIEW_OUTPUT" | sed '/^[[:space:]]*$/d' | tail -1)
VERDICT=$(echo "$LAST_LINE" | sed -n 's/.*VERDICT:[[:space:]]*\(PASS\|FAIL\).*/\1/p')

if [ "$VERDICT" = "PASS" ]; then
  echo "---"
  echo "Gate result: PASSED"
  exit 0
elif [ "$VERDICT" = "FAIL" ]; then
  echo "---"
  echo "Gate result: FAILED"
  show_agent_stderr
  exit 1
else
  echo "---" >&2
  echo "ERROR: Could not extract VERDICT from review output. Treating as failure." >&2
  show_agent_stderr
  exit 1
fi
