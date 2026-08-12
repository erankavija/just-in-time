#!/usr/bin/env bash
set -euo pipefail

# install-jit — install the `jit` CLI from this checkout.
#
# `cargo install --path crates/jit`, resolved relative to this script rather
# than the caller's working directory, so it works from anywhere inside the
# repository.
#
# Usage:
#   scripts/install-jit.sh [extra cargo install args]
# e.g.
#   scripts/install-jit.sh              # install crates/jit to ~/.cargo/bin
#   scripts/install-jit.sh --force      # forwarded to cargo install

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

exec cargo install --path "$repo_root/crates/jit" "$@"
