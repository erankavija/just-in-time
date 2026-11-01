#!/usr/bin/env bash
# Land a branch on this checkout: under the repository write lock, commit all
# pending tracker state as its own neutral commit, then merge with a merge commit.
#
# A conflict aborts the merge, keeps the sync commit, names the paths, and exits
# nonzero; resolve it with `git merge` by hand. Commit hooks run.
#
# Usage: land.sh <branch>
set -euo pipefail

SYNC_MESSAGE='chore(jit): sync tracker'
# Everything under .jit except its machine-local runtime state.
TRACKER=(.jit ':(exclude,glob).jit/.*.lock' ':(exclude).jit/tmp'
    ':(exclude).jit/worktree.json' ':(exclude).jit/server.log' ':(exclude).jit/server.pid.json')

refuse() { echo "land: $*" >&2; exit 1; }

land() { # branch
    local foreign conflicts
    git rev-parse --verify --quiet --end-of-options "$1^{commit}" > /dev/null ||
        refuse "'$1' names no commit"
    foreign="$(git status --short -uall -- . ':(exclude).jit' ':(exclude,glob)**/.jit-bootstrap.lock')"
    [[ -z "$foreign" ]] || refuse "uncommitted changes outside .jit:"$'\n'"$foreign"
    if [[ -n "$(git status --short -uall -- "${TRACKER[@]}")" ]]; then
        git ls-files -z --modified --deleted --others --exclude-standard -- "${TRACKER[@]}" |
            git add --pathspec-from-file=- --pathspec-file-nul
        git commit -q -m "$SYNC_MESSAGE" -- .jit
    fi
    git merge -q --no-ff --no-edit --end-of-options "$1" && return
    conflicts="$(git diff --name-only --diff-filter=U)"
    [[ -n "$conflicts" ]] || refuse "merging $1 failed"
    git merge --abort
    refuse "merging $1 conflicted and was aborted, in:"$'\n'"$conflicts"
}

[[ $# -ge 1 && $# -le 2 ]] || refuse "usage: land.sh <branch>"
if [[ "$1" == --locked ]]; then
    land "$2"
    exit
fi
[[ $# -eq 1 ]] || refuse "usage: land.sh <branch>"
[[ -z "${JIT_DATA_DIR:-}" ]] || refuse "JIT_DATA_DIR is set, but this script commits .jit only"
root="$(git rev-parse --show-toplevel 2> /dev/null)" || refuse "requires a git repository"
cd "$root"
exec jit worktree exec -- bash "${BASH_SOURCE[0]}" --locked "$1"
