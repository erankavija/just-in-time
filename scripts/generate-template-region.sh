#!/usr/bin/env bash
set -euo pipefail

# generate-template-region — render this repository's graph-template
# declaration from the packaged jit-dogfood template contributions.
#
# The target holds one region, bounded by markers in TOML's own comment syntax:
#   .jit/templates.toml
#     # jit:plan-template:begin  …  # jit:plan-template:end
#
# Everything outside the markers is preserved byte for byte; nothing inside them
# is authored. The registry file's header, and the rationale for the container
# anchor's whole-repository validation gate, therefore live above the markers.
#
# WHERE THE VALUES COME FROM. The plan-before-fan-out bracket is declared as a
# template contribution of the embedded jit-dogfood package, and this
# repository's registry consumes it, so the package is the authority and the
# registry block is generated from it (`@/issue/e204e63d/decision/D-1`).
#
# WHY THIS NEEDS NO PROVENANCE CHECK. Its sibling
# generate-shipped-policy-regions.sh establishes that the jit binary on PATH was
# built from this repository before it writes, because the classification it
# renders can only be read out of an already-installed binary. Nothing here is
# read from an installed binary: cargo compiles the package embedded in this
# checkout as part of running the renderer, so the declarations rendered are the
# ones the working tree declares.
#
# Usage:
#   generate-template-region.sh    (takes no arguments — the target is named above)
#
# Exit codes:
#   0 — the region holds the packaged declarations
#   1 — the render or the publication failed
#   2 — a usage or environment error

# shellcheck source=scripts/regenerate-lib.sh
. "$(cd "$(dirname "$0")" && pwd)/regenerate-lib.sh"
regenerate_require_no_arguments "$@"
regenerate_artifact template-region
