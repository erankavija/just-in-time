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

Copy what you need into your repository and adapt it. [How-To: Custom
Gates](../docs/how-to/custom-gates.md) is where these are wired up: [the AI
review script](../docs/how-to/custom-gates.md#example-ai-review-script) as a
gate checker and its reviewer command, and [the prompt
library](../docs/how-to/custom-gates.md#prompt-library) as `--prompt-file`
context.
