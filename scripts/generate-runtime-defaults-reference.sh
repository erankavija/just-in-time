#!/usr/bin/env bash
set -euo pipefail

# generate-runtime-defaults-reference — render the committed runtime-defaults
# reference from the constants the runtime coordinates with.
#
# Target:
#   docs/reference/runtime-defaults.md
#
# WHERE THE VALUES COME FROM. The lock timeout and poll interval, the orphaned
# temp-file cleanup threshold, and the claim lease TTL are declared once as the
# constants in crates/jit/src/runtime_defaults.rs, which every call site reads,
# so the page cannot state a default the code does not use
# (`@/inv/single-source-prose`). The conformance test beside the constants
# asserts the committed page equals the render and names this script.
#
# Usage:
#   generate-runtime-defaults-reference.sh    (takes no arguments — the target is named above)
#
# Exit codes:
#   0 — the reference holds the constants' current projection
#   1 — the render or the publication failed
#   2 — a usage or environment error

# shellcheck source=scripts/regenerate-lib.sh
. "$(cd "$(dirname "$0")" && pwd)/regenerate-lib.sh"
regenerate_require_no_arguments "$@"
regenerate_artifact runtime-defaults-reference
