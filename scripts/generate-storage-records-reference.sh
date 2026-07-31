#!/usr/bin/env bash
set -euo pipefail

# generate-storage-records-reference — render the committed storage-record
# reference from the record definitions the binary writes with.
#
# Target:
#   docs/reference/storage-records.md
#
# WHERE THE VALUES COME FROM. The identifier widths come from the constants that
# enforce them, the event-log sample lines from the tag samples encoded the way
# the repository-state finalizer publishes them, and the gate-run record from
# the type schemars derives its field list from — all declared in
# crates/jit/src/storage/reference.rs and its cited sources, so the page is
# generated from the definitions rather than restating them
# (`@/inv/single-source-prose`). The conformance test beside the render asserts
# the committed page equals it and names this script.
#
# Usage:
#   generate-storage-records-reference.sh    (takes no arguments — the target is named above)
#
# Exit codes:
#   0 — the reference holds the definitions' current projection
#   1 — the render or the publication failed
#   2 — a usage or environment error

. "$(cd "$(dirname "$0")" && pwd)/regenerate-lib.sh"
regenerate_require_no_arguments "$@"
regenerate_artifact storage-records-reference
