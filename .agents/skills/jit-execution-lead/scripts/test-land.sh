#!/usr/bin/env bash
# Test suite for land.sh, over fixture repositories it owns.
#
# `jit` comes from PATH and must carry `jit worktree exec`.
#
# Exit 0 when all assertions pass; 1 otherwise; 2 for a missing tool.

# Not `-e`: assertions inspect command output rather than aborting on it.
set -uo pipefail

PASS=0; FAIL=0

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAND="$SCRIPT_DIR/land.sh"

for tool in git jit jq; do
    command -v "$tool" > /dev/null 2>&1 || { echo "ERROR: '$tool' not on PATH"; exit 2; }
done
jit worktree exec --help > /dev/null 2>&1 || { echo "ERROR: jit lacks 'worktree exec'"; exit 2; }

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null

FIX="$(mktemp -d)"
# shellcheck disable=SC2329  # invoked via trap
cleanup() { rm -rf "$FIX"; }
trap cleanup EXIT

assert() { # name, condition...
    local name="$1"; shift
    if "$@"; then
        PASS=$((PASS + 1)); echo "PASS: $name"
    else
        FAIL=$((FAIL + 1)); echo "FAIL: $name"
    fi
}

equal() { [[ "$1" == "$2" ]]; }
contains() { [[ "$1" == *"$2"* ]]; }
lacks() { [[ "$1" != *"$2"* ]]; }
empty() { [[ -z "$1" ]]; }

issue() { jit issue create --title "$1" --json | jq -r .id; }
tree_titles() { git show "$1:.jit/events.jsonl" | jq -r 'select(.title) | .title' | sort | xargs; }

# A repository with a committed tracker, and a branch adding one code file.
new_repo() { # name
    local dir="$FIX/$1"
    git init -q -b main "$dir"
    cd "$dir" || exit 2
    git config user.email test@example.com
    git config user.name Test
    git config commit.gpgsign false
    jit init > /dev/null
    printf '.jit-bootstrap.lock\n.jit/.*.lock\n.jit/worktree.json\n' > .gitignore
    git add -A
    git commit -qm base
    git checkout -qb side
    echo side > side.txt
    git add side.txt
    git commit -qm side
    git checkout -q main
}

land_status() { # branch; sets OUT and STATUS
    OUT="$("$LAND" "$1" 2>&1)"; STATUS=$?
}

# --- REQ-01: pending tracker state lands in its own neutral commit
new_repo neutral
issue "Pending from another session" > /dev/null
land_status side
assert "land succeeds" equal "$STATUS" 0
assert "first parent is the sync commit" equal "$(git log -1 --format=%s HEAD^1)" 'chore(jit): sync tracker'
assert "second parent is the branch" equal "$(git rev-parse HEAD^2)" "$(git rev-parse side)"
assert "the merge adds no tracker path" empty "$(git diff --name-only HEAD^1 HEAD -- .jit)"
assert "the tree is clean" empty "$(git status --short)"

# --- REQ-02: the sync commit is coherent, and a leftover record refuses
new_repo coherent
first="$(issue One)"; second="$(issue Two)"
land_status side
ids="$(printf '%s\n%s\n' "$first" "$second" | sort | xargs)"
assert "issue records hold the pending ids" equal \
    "$(git ls-tree --name-only HEAD^1 .jit/issues/ | xargs -n1 basename | sed 's/\.json$//' | sort | xargs)" "$ids"
assert "events hold the pending ids" equal \
    "$(git show HEAD^1:.jit/events.jsonl | jq -r 'select(.type == "issue_created") | .issue_id' | sort | xargs)" "$ids"

new_repo leftover
issue Pending > /dev/null
mkdir -p .jit/tmp
echo leftover > .jit/tmp/redo.record
prior="$(git rev-parse HEAD)"
land_status side
assert "a leftover record refuses" equal "$STATUS" 1
assert "the refusal names the record" contains "$OUT" "redo record"
assert "a refusal commits nothing" equal "$(git rev-parse HEAD)" "$prior"
assert "the record is not applied" equal "$(cat .jit/tmp/redo.record)" leftover

# A mutation during the sync commit fails its bounded wait, and one that outlasts
# the landing stays out of its commits.
new_repo concurrent
issue Pending > /dev/null
cat > .git/hooks/pre-commit <<'HOOK'
#!/bin/sh
unset GIT_INDEX_FILE GIT_DIR GIT_WORK_TREE
JIT_LOCK_TIMEOUT=1 jit issue create --title During > /dev/null 2>&1
echo $? > .git/during.status
(JIT_LOCK_TIMEOUT=60 jit issue create --title After > /dev/null 2>&1
 echo $? > .git/after.tmp; mv .git/after.tmp .git/after.status) < /dev/null > /dev/null 2>&1 &
HOOK
chmod +x .git/hooks/pre-commit
land_status side
for _ in $(seq 1 600); do [[ -e .git/after.status ]] && break; sleep 0.1; done
assert "a mutation under the lock times out" equal "$(cat .git/during.status)" 11
assert "a queued mutation completes after the landing" equal "$(cat .git/after.status)" 0
assert "neither mutation is in the sync commit" equal "$(tree_titles HEAD^1)" Pending
assert "neither mutation is in the merge commit" equal "$(tree_titles HEAD)" Pending
assert "the queued mutation stays pending" contains "$(git status --short)" .jit/events.jsonl

# --- REQ-03: the landing's contract
new_repo clean
prior="$(git rev-parse HEAD)"
land_status side
assert "nothing pending makes no sync commit" equal "$(git rev-parse HEAD^1)" "$prior"

new_repo union
branch_side="$(issue "Updated on the branch")"; main_side="$(issue "Updated on main")"
git add .jit
git commit -qm base-issues
base_lines="$(wc -l < .jit/events.jsonl)"
git checkout -q side
git merge -q --no-edit main
jit issue update "$branch_side" --state "done" > /dev/null
git add .jit
git commit -qm side-tracker
git checkout -q main
jit issue update "$main_side" --state "done" > /dev/null
land_status side
assert "both sides appended to the log and it merged" equal "$STATUS" 0
assert "the union driver keeps both sides' event lines" equal \
    "$(git show HEAD:.jit/events.jsonl | tail -n +$((base_lines + 1)) | jq -r .issue_id | sort -u | xargs)" \
    "$(printf '%s\n%s\n' "$branch_side" "$main_side" | sort | xargs)"
assert "the union merge leaves no markers" empty "$(git grep -l '^<<<<<<<' HEAD -- .jit/events.jsonl)"

new_repo conflict
contested="$(issue Contested)"
git add .jit
git commit -qm contested-issue
git checkout -q side
git merge -q --no-edit main
jit issue update "$contested" --priority high > /dev/null
git add .jit
git commit -qm branch-edit
git checkout -q main
jit issue update "$contested" --priority low > /dev/null
prior="$(git rev-parse HEAD)"
land_status side
assert "a conflict exits nonzero" equal "$STATUS" 1
assert "a conflict names the path" contains "$OUT" ".jit/issues/$contested.json"
assert "a conflict keeps the sync commit" equal "$(git rev-parse HEAD^)" "$prior"
assert "the sync commit is HEAD" equal "$(git log -1 --format=%s)" 'chore(jit): sync tracker'
assert "the merge is aborted" empty "$(git status --short)"
assert "no merge is in progress" lacks "$(ls .git)" MERGE_HEAD

new_repo dirty
issue Pending > /dev/null
echo local > uncommitted.txt
prior="$(git rev-parse HEAD)"
land_status side
assert "a dirty path outside .jit refuses" equal "$STATUS" 1
assert "the refusal names the path" contains "$OUT" uncommitted.txt
assert "the dirty refusal commits nothing" equal "$(git rev-parse HEAD)" "$prior"

new_repo typo
issue Pending > /dev/null
prior="$(git rev-parse HEAD)"
land_status no-such-branch
assert "an unknown branch refuses" equal "$STATUS" 1
assert "the refusal names the branch" contains "$OUT" no-such-branch
assert "an unknown branch makes no sync commit" equal "$(git rev-parse HEAD)" "$prior"

new_repo data-dir
issue Pending > /dev/null
prior="$(git rev-parse HEAD)"
OUT="$(JIT_DATA_DIR="$PWD/.jit" "$LAND" side 2>&1)"; STATUS=$?
assert "JIT_DATA_DIR refuses" equal "$STATUS" 1
assert "the refusal names JIT_DATA_DIR" contains "$OUT" JIT_DATA_DIR
assert "JIT_DATA_DIR commits nothing" equal "$(git rev-parse HEAD)" "$prior"

new_repo no-entry
issue Pending > /dev/null
prior="$(git rev-parse HEAD)"
pending="$(git status --short)"
OUT="$("$LAND" --locked side 2>&1)"; STATUS=$?
assert "a direct body entry refuses" equal "$STATUS" 1
assert "the refusal is the usage" contains "$OUT" usage
assert "a direct body entry commits nothing" equal "$(git rev-parse HEAD)" "$prior"
assert "a direct body entry stages nothing" equal "$(git status --short)" "$pending"

new_repo refused-hook
issue Pending > /dev/null
printf '#!/bin/sh\necho hook refused >&2\nexit 1\n' > .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit
prior="$(git rev-parse HEAD)"
land_status side
assert "a refusing hook fails the landing" equal "$STATUS" 1
assert "the hook's refusal is loud" contains "$OUT" "hook refused"
assert "a refusing hook leaves HEAD" equal "$(git rev-parse HEAD)" "$prior"

mkdir "$FIX/no-git"
cd "$FIX/no-git" || exit 2
jit init > /dev/null
land_status side
assert "outside git refuses" equal "$STATUS" 1
assert "the refusal names git" contains "$OUT" "git repository"

new_repo busy
issue Pending > /dev/null
prior="$(git rev-parse HEAD)"
JIT_LOCK_TIMEOUT=1 jit worktree exec -- sh -c \
    "JIT_LOCK_TIMEOUT=1 \"$LAND\" side > .git/nested.out 2>&1; echo \$? > .git/nested.status"
assert "a held lock times out loudly" equal "$(cat .git/nested.status)" 11
assert "the timeout names the lock" contains "$(cat .git/nested.out)" "Lock timeout"
assert "a timeout commits nothing" equal "$(git rev-parse HEAD)" "$prior"

echo
echo "passed: $PASS failed: $FAIL"
[[ "$FAIL" -eq 0 ]]
