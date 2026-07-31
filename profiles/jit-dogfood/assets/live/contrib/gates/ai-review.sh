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
#   your-reviewer-command                   # configure inspection-only mode
#   cat                                    # dry-run (echoes the prompt)
#
# Setup:
#   1. Applying the profile installs this script, executable, at
#      contrib/gates/ai-review.sh. Working from a source checkout instead,
#      copy it there yourself and `chmod +x contrib/gates/ai-review.sh`.
#   2. Define the gate:
#        jit gate define ai-review \
#          --title "AI Code Review" \
#          --description "AI-powered code review" \
#          --mode auto --stage postcheck \
#          --pass-context \
#          --prompt "Review the implementation for correctness and style." \
#          --checker-command "./contrib/gates/ai-review.sh" \
#          --env REVIEWER_AGENT="your-reviewer-command" \
#          --timeout 120
#   3. Run: jit gate evaluate <issue> ai-review

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
  echo "  Example: REVIEWER_AGENT='your-reviewer-command'" >&2
  exit 1
fi

if ! grep -Eq '"prompt"[[:space:]]*:[[:space:]]*"[^"]+' "$JIT_CONTEXT_FILE"; then
  echo "ERROR: No prompt defined for this gate. Set --prompt or --prompt-file when defining the gate." >&2
  exit 1
fi

# Capture agent stderr to a temp file so we can surface it on errors.
AGENT_STDERR=$(mktemp)
trap 'rm -f "$AGENT_STDERR"' EXIT

# Feed the complete context directly. The JSON contains the resolved `prompt`
# plus the issue, gate, and prior-run data, so no external JSON parser is
# required on the adopter host.
REVIEW_OUTPUT=$(
  {
    cat <<'EOF'
Read the complete JIT gate context below. Follow its top-level `prompt` field as
the review policy and use the remaining fields as evidence.

## Context

```json
EOF
    cat "$JIT_CONTEXT_FILE"
    cat <<'EOF'
```

Before the verdict line, output a numbered list of every finding across all categories, followed by a single line stating the total count (e.g., "Total findings: N"). All findings must appear in this single enumeration — none may be withheld for a later round.

Then emit a machine-readable findings block so jit can consume the findings as data. The block is two line-exact fence markers wrapping a single JSON object:

<<<JIT-FINDINGS-JSON
{"verdict":"fail","summary":"<one line>","findings":[{"id":"F1","severity":"high","summary":"<one line>","file":"path/to/file.rs","line":42,"references":["<qualified-policy-id>"]}]}
JIT-FINDINGS-JSON>>>

Rules for the block:
- \`verdict\` is "pass" or "fail" and MUST match the VERDICT line below.
- \`findings\` lists every finding from the numbered list above, in order. Use an empty array when there are none; when the checker-specific policy distinguishes blocking from advisory feedback, a passing verdict may include advisory findings.
- \`severity\` is one of "high", "medium", "low". \`disposition\` (blocking or advisory) and \`origin\` (issue-impact or pre-existing) are optional classifications that checker-specific policies may require. \`file\` and \`line\` are optional; omit them when a finding is not tied to a specific location. \`references\` is an optional array of policy identifiers and may be omitted when no policy reference governs the finding.
- Emit valid JSON on a single line. Do not wrap the block in a code fence.

You MUST end your response with exactly one of these lines:
VERDICT: PASS
VERDICT: FAIL
No text may follow the verdict line.
EOF
  } | eval "$REVIEWER_AGENT" 2>"$AGENT_STDERR"
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

# Extract the last line that matches VERDICT: PASS or VERDICT: FAIL (portable —
# works on BSD and GNU). Scanning for the last *matching* line rather than the
# last non-blank line makes the parser robust to reviewer prose that follows the
# verdict line. Rule: if both PASS and FAIL appear, the last verdict line wins.
# No matching line → verdict-unparseable → treated as failure.
VERDICT=$(echo "$REVIEW_OUTPUT" | grep -E 'VERDICT:[[:space:]]*(PASS|FAIL)' | tail -1 | sed -n 's/.*VERDICT:[[:space:]]*\(PASS\|FAIL\).*/\1/p')

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
