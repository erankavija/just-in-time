#!/usr/bin/env bash
set -euo pipefail

# generate-gate-presets-reference — render the committed gate-preset reference
# from the presets the binary ships.
#
# Target:
#   docs/reference/gate-presets.md
#
# WHERE THE VALUES COME FROM. The preset definitions are the authority: the
# built-in presets loaded in crates/jit/src/gate_presets/ supply each preset's
# name and description and each bundled gate's key, title, stage, mode,
# description, and checker configuration (`@/inv/single-source-prose`). The
# render sorts presets by name and checker environment variables by key, so no
# map iteration order reaches the page. The conformance test beside the render
# asserts the committed page equals it and names this script.
#
# Usage:
#   generate-gate-presets-reference.sh    (takes no arguments — the target is named above)
#
# Exit codes:
#   0 — the reference holds the presets' current projection
#   1 — the render or the publication failed
#   2 — a usage or environment error

# shellcheck source=scripts/regenerate-lib.sh
. "$(cd "$(dirname "$0")" && pwd)/regenerate-lib.sh"
regenerate_require_no_arguments "$@"
regenerate_artifact gate-presets-reference
