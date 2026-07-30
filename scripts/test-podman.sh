#!/usr/bin/env bash
set -euo pipefail

# test-podman — build and runtime-smoke the supported server image with Podman.
#
# The image built from the repository-root Dockerfile is the whole supported
# topology: one `jit-server` process serving the API and the built web UI on
# port 3000, against a whole repository bind-mounted at /repo.
#
# Identity. The image runs as UID:GID 10001:10001, a fixed numeric default that
# owns nothing on the host, so an unmapped container can only serve a mount that
# 10001 may read, write, and search. A deployment maps the repository owner onto
# the container identity instead:
#
#   JIT_UID=$(stat -c '%u' /path/to/repo)
#   JIT_GID=$(stat -c '%g' /path/to/repo)
#   podman run --userns keep-id --user "$JIT_UID:$JIT_GID" ...
#
# docs/how-to/deployment.md is the canonical account of that contract; this
# script prints the two values for the repository it is pointed at, so the smoke
# and the deployment speak about the same identity.
#
# The smoke itself is the container contract suite, which builds the image and
# exercises both identity arrangements, the API and SPA routes, repository
# document search, the bounded shutdown drain, and the refusal to start against
# an uninitialized mount.
#
# Usage:
#   test-podman.sh [REPO]
# REPO is the repository whose ownership is reported; it defaults to this
# repository. The suite serves fixtures it creates in temporary directories, so
# REPO is never mounted or written to.
#
# Exit codes:
#   0 — the image builds and every contract test passes
#   1 — a contract test failed
#   2 — environment error: Podman is unavailable, the `jit` the fixtures are
#       seeded with is not on PATH, or REPO is not a directory

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_REPO="${1:-$ROOT}"

if ! command -v podman >/dev/null 2>&1; then
    echo "test-podman: podman is not installed" >&2
    exit 2
fi

if ! command -v "${JIT_BIN:-jit}" >/dev/null 2>&1; then
    echo "test-podman: the suite seeds its fixtures with '${JIT_BIN:-jit}', which is not on PATH" >&2
    echo "test-podman: build it with 'cargo build --bin jit', or set JIT_BIN" >&2
    exit 2
fi

if [ ! -d "$TARGET_REPO" ]; then
    echo "test-podman: '$TARGET_REPO' is not a directory" >&2
    exit 2
fi

echo "podman: $(podman --version)"
echo "repository the reported identity is derived from: $TARGET_REPO"
printf 'mount identity: JIT_UID=%s JIT_GID=%s (the image defaults to 10001:10001)\n' \
    "$(stat -c '%u' "$TARGET_REPO")" \
    "$(stat -c '%g' "$TARGET_REPO")"
echo

CONTAINER_ENGINE=podman JIT_IMAGE_RUNTIME=1 \
    python3 "$ROOT/test-vectors/container-image/test_image_contract.py"
