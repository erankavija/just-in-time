#!/usr/bin/env bash
set -euo pipefail

# generate-invariant-region — render this repository's generated invariant
# declarations from the packaged jit-dogfood invariant contributions.
#
# The target holds one region, bounded by markers in TOML's own comment syntax:
#   .jit/invariants.toml
#     # jit:packaged-invariants:begin  …  # jit:packaged-invariants:end
#
# Everything outside the markers is preserved byte for byte; nothing inside them
# is authored. Invariants this repository holds apart from the package live
# outside the markers.
#
# Usage:
#   generate-invariant-region.sh    (takes no arguments — the target is named above)
#
# Exit codes:
#   0 — the region holds the packaged declarations
#   1 — the render or the publication failed
#   2 — a usage or environment error

# shellcheck source=scripts/regenerate-lib.sh
. "$(cd "$(dirname "$0")" && pwd)/regenerate-lib.sh"
regenerate_require_no_arguments "$@"
regenerate_artifact invariant-region
