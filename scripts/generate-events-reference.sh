#!/usr/bin/env bash
set -euo pipefail

# generate-events-reference — render the committed event-log reference from the
# event catalog.
#
# Target:
#   docs/reference/events.md
#
# WHERE THE VALUES COME FROM. Every row — tag, scope, whether the record carries
# an issue id, and the description — is derived from the `EventTag` vocabulary
# declared in crates/jit/src/domain/event_catalog.rs, so the page is a
# projection of the type the runtime writes with rather than a hand-copy of it
# (`@/inv/single-source-prose`). The conformance test beside that declaration
# asserts the committed page equals the render and names this script.
#
# Usage:
#   generate-events-reference.sh    (takes no arguments — the target is named above)
#
# Exit codes:
#   0 — the reference holds the catalog's current projection
#   1 — the render or the publication failed
#   2 — a usage or environment error

. "$(cd "$(dirname "$0")" && pwd)/regenerate-lib.sh"
regenerate_require_no_arguments "$@"
regenerate_artifact events-reference
