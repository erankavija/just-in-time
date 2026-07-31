#!/usr/bin/env bash
set -euo pipefail

# generate-exit-code-reference — render the committed exit-code reference from
# the process exit-code taxonomy.
#
# Target:
#   docs/reference/exit-codes.md
#
# WHERE THE VALUES COME FROM. The page projects the global taxonomy and the
# per-command mappings the command schema declares in crates/jit/src/schema.rs —
# the same declarations `jit --schema` publishes — so a script branching on an
# exit code reads what the binary returns (`@/inv/single-source-prose`). Tests
# beside the declaration bind each mapping to the runtime and assert the
# committed page equals the render, naming this script.
#
# Usage:
#   generate-exit-code-reference.sh    (takes no arguments — the target is named above)
#
# Exit codes:
#   0 — the reference holds the taxonomy's current projection
#   1 — the render or the publication failed
#   2 — a usage or environment error

. "$(cd "$(dirname "$0")" && pwd)/regenerate-lib.sh"
regenerate_require_no_arguments "$@"
regenerate_artifact exit-code-reference
