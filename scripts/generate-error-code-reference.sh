#!/usr/bin/env bash
set -euo pipefail

# generate-error-code-reference — render the committed error-code reference from
# the machine-readable error vocabulary.
#
# Target:
#   docs/reference/error-codes.md
#
# WHERE THE VALUES COME FROM. Each row reads its wire spelling, meaning, and
# numeric exit status off the `ErrorCode` vocabulary declared in
# crates/jit/src/output.rs, so an adopter reads the same codes the runtime emits
# (`@/inv/single-source-prose`). A derive-checked member list rejects an unlisted
# variant, and the conformance test beside the declaration asserts the committed
# page equals the render and names this script.
#
# Usage:
#   generate-error-code-reference.sh    (takes no arguments — the target is named above)
#
# Exit codes:
#   0 — the reference holds the vocabulary's current projection
#   1 — the render or the publication failed
#   2 — a usage or environment error

. "$(cd "$(dirname "$0")" && pwd)/regenerate-lib.sh"
regenerate_require_no_arguments "$@"
regenerate_artifact error-code-reference
