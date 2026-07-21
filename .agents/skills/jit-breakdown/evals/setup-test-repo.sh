#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
exec "$here/../../jit-planning-lead/evals/setup-test-repo.sh" "$@"
