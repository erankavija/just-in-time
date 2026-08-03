#!/usr/bin/env bash
set -euo pipefail

# generate-gate-presets-reference — render the committed gate-preset reference
# from the preset contract and the portable checker syntax the crate carries.
#
# Target:
#   docs/reference/gate-presets.md
#
# WHERE THE VALUES COME FROM. The render in
# crates/jit/src/gate_presets/reference.rs is the authority: the projected
# `.jit/gates.toml` syntax block is the constant the loader test parses, so the
# page cannot state a syntax the gate registry would reject
# (`@/inv/single-source-prose`). The conformance test beside the render asserts
# the committed page equals it and names this script.
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
