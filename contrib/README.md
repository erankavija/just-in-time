# contrib/

Community-contributed scripts and resources for jit. These are not part of the core CLI but provide useful building blocks for common workflows.

## Contents

### `gates/`

Scripts and prompt templates for gate checkers.

- **`ai-review.sh`** — Production-ready AI review gate. Pipes gate context into an AI agent CLI and parses a VERDICT from the output. See the script header for setup instructions.

- **`prompts/`** — Ready-to-use prompt templates for context-aware gates:
  - `code-review.md` — General code review (correctness, style, error handling)
  - `security-audit.md` — OWASP Top 10 security checklist with severity ratings
  - `test-adequacy.md` — Test coverage evaluation against requirements

## Usage

Copy what you need into your repo and adapt:

```bash
# Copy the AI review script
cp contrib/gates/ai-review.sh scripts/
chmod +x scripts/ai-review.sh

# Define a gate using a contrib prompt
jit gate define ai-review \
  --title "AI Code Review" \
  --description "AI-powered code review" \
  --mode auto --stage postcheck \
  --pass-context \
  --prompt-file "contrib/gates/prompts/code-review.md" \
  --checker-command "./scripts/ai-review.sh" \
  --env REVIEWER_AGENT="copilot -s --model claude-haiku-4.5" \
  --timeout 120
```

See [How-To: Custom Gates](docs/how-to/custom-gates.md) for full documentation.
