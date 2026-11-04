#!/usr/bin/env bash
# Test suite for the cache seed in dispatch-worker-worktree.sh and the harvest
# in reclaim-worker-worktree.sh, over a fixture repository and pool it owns.
#
# Exit 0 when all assertions pass; 1 otherwise; 2 for a missing tool.

# Not `-e`: assertions inspect command output rather than aborting on it.
set -uo pipefail

PASS=0; FAIL=0

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH="$SCRIPT_DIR/dispatch-worker-worktree.sh"
RECLAIM="$SCRIPT_DIR/reclaim-worker-worktree.sh"

for tool in git rsync stat diff tar; do
    command -v "$tool" > /dev/null 2>&1 || { echo "ERROR: '$tool' not on PATH"; exit 2; }
done

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE

FIX="$(mktemp -d)"
export LEAD_WORKTREE_TMP_ROOT="$FIX/worker-tmp-root"
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

same_inode() { [[ "$(stat -c %i "$1")" == "$(stat -c %i "$2")" ]]; }
different_inode() { ! same_inode "$1" "$2"; }
same_mtime() { [[ "$(stat -c %Y "$1")" == "$(stat -c %Y "$2")" ]]; }
has_content() { [[ "$(cat "$1")" == "$2" ]]; }

new_repo() { # dir
    git init -q -b main "$1"
    echo 'target/' > "$1/.gitignore"
    git -C "$1" add .gitignore
    git -C "$1" -c user.email=test@example.com -c user.name=Test \
        -c commit.gpgsign=false commit -qm init
}

put() { # path, content
    mkdir -p "$(dirname "$1")"
    printf '%s\n' "$2" > "$1"
}

new_pool() { # dir
    local pool="$1" rel
    for rel in \
        target/CACHEDIR.TAG \
        target/.rustc_info.json \
        target/debug/.cargo-lock \
        target/debug/tmp/.jit/issues/leftover.json \
        target/debug/deps/libpkg.rlib \
        target/debug/deps/build \
        target/debug/deps/pkg-1.d \
        target/debug/pkg.d \
        target/debug/conf.d/settings \
        target/debug/build/pkg-1/output \
        target/debug/.fingerprint/pkg-1/dep-lib \
        target/release/build/pkg-2/out/build/generated.rs \
        target/release/.fingerprint/pkg-2/invoked.timestamp \
        target/debug/incremental/pkg/.fingerprint/nested \
        node_modules/pkg/index.js \
        node_modules/pkg/build/addon.node \
        build/tool.o; do
        put "$pool/$rel" "pool $rel"
        touch -d '2020-01-02 03:04:05' "$pool/$rel"
    done
    ln "$pool/target/debug/build/pkg-1/output" "$pool/target/debug/build/pkg-1/output-alias"
    ln -s libpkg.rlib "$pool/target/debug/deps/.link"
}

is_mutable_state() { [[ "/$1" == */.* || "/$1" == */build/* || "$1" == *.d ]]; }

# --- Hardlink seed --------------------------------------------------------
repo="$FIX/seed"; pool="$FIX/seed-pool"; snapshots="$FIX/seed-snapshots"
new_repo "$repo"; new_pool "$pool"; mkdir -p "$snapshots"
out="$(cd "$repo" && LEAD_CACHE_POOL="$pool" LEAD_SNAPSHOT_DIR="$snapshots" "$DISPATCH" a 2>&1)"
assert "dispatch succeeds over a pool" test "$?" -eq 0
wt="$repo/.agents/worktrees/agent-a"
assert "the seed reports hardlink mode" grep -q 'target seeded from pool (hardlink)' <<<"$out"
assert "the snapshot lands in the given directory" test -e "$snapshots/lead-pre-dispatch-latest.txt"

while IFS= read -r rel; do
    if is_mutable_state "$rel"; then
        assert "$rel shares no inode with the pool" different_inode "$pool/$rel" "$wt/$rel"
    else
        assert "$rel stays hardlinked to the pool" same_inode "$pool/$rel" "$wt/$rel"
    fi
    assert "$rel keeps its mtime" same_mtime "$pool/$rel" "$wt/$rel"
done < <(cd "$pool" && find . -type f -printf '%P\n')

assert "a dot-named symlink shares no inode with the pool" different_inode \
    "$pool/target/debug/deps/.link" "$wt/target/debug/deps/.link"
assert "the seed matches the pool's content" diff -r "$pool/target" "$wt/target"
assert "a hardlink inside copied build state survives" same_inode \
    "$wt/target/debug/build/pkg-1/output" "$wt/target/debug/build/pkg-1/output-alias"

# --- Copy fallback --------------------------------------------------------
repo="$FIX/fallback"; pool="$FIX/fallback-pool"
new_repo "$repo"; new_pool "$pool"
shim="$FIX/no-hardlink-bin"; mkdir -p "$shim"
real_cp="$(command -v cp)"
cat > "$shim/cp" <<SHIM
#!/usr/bin/env bash
[[ "\$1" == "-al" ]] && exit 1
exec "$real_cp" "\$@"
SHIM
chmod +x "$shim/cp"
out="$(cd "$repo" && PATH="$shim:$PATH" LEAD_CACHE_POOL="$pool" LEAD_SNAPSHOT_DIR="$snapshots" "$DISPATCH" a 2>&1)"
assert "dispatch succeeds when hardlinking fails" test "$?" -eq 0
wt="$repo/.agents/worktrees/agent-a"
assert "the seed reports copy mode" grep -q 'target seeded from pool (copy)' <<<"$out"
assert "the copied seed matches the pool's content" diff -r "$pool/target" "$wt/target"
assert "a copied seed shares no inode with the pool" different_inode \
    "$pool/target/debug/deps/libpkg.rlib" "$wt/target/debug/deps/libpkg.rlib"

# --- Harvest --------------------------------------------------------------
no_rsync="$FIX/no-rsync-bin"; mkdir -p "$no_rsync"
for tool in bash git cp mkdir basename rm find sort tar xargs; do
    ln -s "$(command -v "$tool")" "$no_rsync/$tool"
done

check_harvest() { # branch, PATH
    local branch="$1" search_path="$2"
    local repo="$FIX/harvest-$branch" pool="$FIX/harvest-$branch-pool" sibling unshared_sibling unshared_inode
    new_repo "$repo"
    (cd "$repo" && LEAD_CACHE_POOL="$FIX/absent-pool" LEAD_SNAPSHOT_DIR="$snapshots" "$DISPATCH" a > /dev/null 2>&1)
    local wt="$repo/.agents/worktrees/agent-a"
    sibling="$FIX/harvest-$branch-sibling"

    put "$pool/target/debug/fresh" 'pool'
    touch -d '2020-01-01' "$pool/target/debug/fresh"
    ln "$pool/target/debug/fresh" "$sibling"
    put "$wt/target/debug/fresh" 'worktree'
    touch -d '2021-01-01' "$wt/target/debug/fresh"

    put "$pool/target/debug/unshared" 'pool'
    touch -d '2020-01-01' "$pool/target/debug/unshared"
    unshared_inode="$(stat -c %i "$pool/target/debug/unshared")"
    unshared_sibling="$FIX/harvest-$branch-unshared-sibling"
    ln "$pool/target/debug/unshared" "$unshared_sibling"
    put "$wt/target/debug/unshared" 'worktree'
    touch -d '2021-01-01' "$wt/target/debug/unshared"

    put "$pool/target/debug/kept" 'pool'
    touch -d '2021-01-01' "$pool/target/debug/kept"
    put "$wt/target/debug/kept" 'worktree'
    touch -d '2020-01-01' "$wt/target/debug/kept"

    mkdir -p "$wt/target/debug/empty"
    touch -d '2017-01-01' "$wt/target/debug/empty"
    local empty_mtime
    empty_mtime="$(stat -c %Y "$wt/target/debug/empty")"
    local profile rel before
    for profile in debug release triple/debug; do
        put "$pool/target/$profile/incremental/old/state" 'old incremental'
        put "$wt/target/$profile/incremental/new/state" 'new incremental'
        for rel in pkg-1111111111111111 pkg-1111111111111111.d \
            libpkg-1111111111111111.rlib libpkg-1111111111111111.rmeta; do
            put "$pool/target/$profile/deps/$rel" 'old pool generation'
            touch -d '2019-01-01' "$pool/target/$profile/deps/$rel"
        done
        for rel in pkg-2222222222222222 pkg-2222222222222222.d \
            libpkg-2222222222222222.rlib libpkg-2222222222222222.rmeta \
            pkg-3333333333333333 pkg-3333333333333333.d \
            libpkg-3333333333333333.rlib libpkg-3333333333333333.rmeta; do
            put "$wt/target/$profile/deps/$rel" 'built from the same sources'
            touch -d '2020-01-01' "$wt/target/$profile/deps/$rel"
        done
        touch -d '2021-01-01' "$wt/target/$profile/deps/pkg-3333333333333333"
        touch -d '2021-01-01' "$wt/target/$profile/deps/libpkg-3333333333333333.rlib"
        # The selected generation keeps its older companions as a unit.
        touch -d '2018-01-01' "$wt/target/$profile/deps/pkg-3333333333333333.d" \
            "$wt/target/$profile/deps/libpkg-3333333333333333.rmeta"
    done
    for rel in libcrate-1111111111111111.rlib crate-1111111111111111.d \
        libcrate-2222222222222222.rlib crate-2222222222222222.d; do
        put "$wt/target/debug/deps/$rel" 'library companion'
        touch -d '2020-01-01' "$wt/target/debug/deps/$rel"
    done
    touch -d '2021-01-01' "$wt/target/debug/deps/libcrate-2222222222222222.rlib"
    touch -d '2018-01-01' "$wt/target/debug/deps/crate-2222222222222222.d"
    put "$wt/target/debug/deps/libpkg-6666666666666666.so" 'dynamic library'
    touch -d '2015-01-01' "$wt/target/debug/deps/libpkg-6666666666666666.so"
    for rel in tied-7777777777777777 tied-8888888888888888; do
        put "$wt/target/debug/deps/$rel" 'tied generation'
        touch -d '2020-01-01' "$wt/target/debug/deps/$rel"
    done
    # A different profile and target retain their own most recent generation.
    put "$pool/target/other/deps/pkg-4444444444444444" 'other profile'
    put "$pool/target/debug/deps/other-4444444444444444" 'other target'

    (cd "$repo" && PATH="$search_path" LEAD_CACHE_POOL="$pool" "$RECLAIM" a > /dev/null 2>&1)
    assert "$branch: reclaim succeeds" test "$?" -eq 0
    assert "$branch: a newer file reaches the pool" has_content "$pool/target/debug/fresh" 'worktree'
    assert "$branch: the harvest leaves a hardlink sibling untouched" has_content "$sibling" 'pool'
    assert "$branch: the harvest replaces an unshared file's inode" \
        test "$(stat -c %i "$pool/target/debug/unshared")" != "$unshared_inode"
    assert "$branch: a newer pool file wins" has_content "$pool/target/debug/kept" 'pool'
    assert "$branch: no incremental directory reaches or remains in the pool" \
        test -z "$(find "$pool" -type d -name incremental -print -quit)"
    assert "$branch: superseded generations from either source are absent" \
        test -z "$(find "$pool" -name 'pkg-1111111111111111*' -o -name 'pkg-2222222222222222*' \
            -o -name 'libpkg-1111111111111111*' -o -name 'libpkg-2222222222222222*')"
    assert "$branch: an empty cache directory survives" test -d "$pool/target/debug/empty"
    assert "$branch: empty directory metadata survives" \
        test "$(stat -c %Y "$pool/target/debug/empty" 2>/dev/null)" = "$empty_mtime"
    assert "$branch: library dep-info follows its library generation" \
        test -f "$pool/target/debug/deps/crate-2222222222222222.d"
    assert "$branch: superseded library dep-info is absent" \
        test ! -e "$pool/target/debug/deps/crate-1111111111111111.d"
    assert "$branch: a dynamic library is separate from rlib and rmeta" \
        test -f "$pool/target/debug/deps/libpkg-6666666666666666.so"
    assert "$branch: tied mtimes keep one deterministic generation" \
        test -f "$pool/target/debug/deps/tied-8888888888888888"
    assert "$branch: the other tied generation is absent" \
        test ! -e "$pool/target/debug/deps/tied-7777777777777777"
    for profile in debug release triple/debug; do
        for rel in pkg-3333333333333333 pkg-3333333333333333.d \
            libpkg-3333333333333333.rlib libpkg-3333333333333333.rmeta; do
            assert "$branch: $profile/$rel survives as a coherent generation" \
                test -f "$pool/target/$profile/deps/$rel"
        done
    done
    assert "$branch: another profile keeps its own generation" \
        test -f "$pool/target/other/deps/pkg-4444444444444444"
    assert "$branch: another target keeps its own generation" \
        test -f "$pool/target/debug/deps/other-4444444444444444"

    before="$(du -sb "$pool" | cut -f1)"
    (cd "$repo" && LEAD_CACHE_POOL="$pool" LEAD_SNAPSHOT_DIR="$snapshots" "$DISPATCH" b > /dev/null 2>&1)
    wt="$repo/.agents/worktrees/agent-b"
    put "$wt/target/debug/incremental/rebuilt/state" 'regenerated'
    for rel in pkg-5555555555555555 pkg-5555555555555555.d \
        libpkg-5555555555555555.rlib libpkg-5555555555555555.rmeta; do
        put "$wt/target/debug/deps/$rel" 'built from the same sources'
        touch -d '2022-01-01' "$wt/target/debug/deps/$rel"
    done
    (cd "$repo" && PATH="$search_path" LEAD_CACHE_POOL="$pool" "$RECLAIM" b > /dev/null 2>&1)
    assert "$branch: reclaim of a repeated build succeeds" test "$?" -eq 0
    assert "$branch: a repeated build does not grow the pool" \
        test "$(du -sb "$pool" | cut -f1)" -le "$before"
}

check_harvest rsync "$PATH"
check_harvest cp "$no_rsync"

# --- Worker temporary directories ---------------------------------------
check_worker_tmp_reclaim() { # label, force
    local label="$1" force="$2"
    local repo="$FIX/worker-tmp-$label" pool="$FIX/worker-tmp-$label-pool"
    local tmp_dir wt dispatch_out
    new_repo "$repo"
    dispatch_out="$(cd "$repo" && LEAD_CACHE_POOL="$pool" \
        LEAD_SNAPSHOT_DIR="$snapshots" "$DISPATCH" a 2>&1)"
    wt="$repo/.agents/worktrees/agent-a"
    tmp_dir="$LEAD_WORKTREE_TMP_ROOT/${repo#/}/agent-a"
    assert "$label: dispatch creates a per-worktree temporary directory" test -d "$tmp_dir"
    assert "$label: prompt exports the temporary directory as TMPDIR" \
        grep -Fq "export TMPDIR=\"$tmp_dir\"" <<<"$dispatch_out"
    mkdir -p "$tmp_dir"
    put "$tmp_dir/worker-leftover" 'left by a killed test'

    if [[ "$force" == 1 ]]; then
        put "$wt/unmerged.txt" 'force reclaim'
        git -C "$wt" add unmerged.txt
        git -C "$wt" -c user.email=test@example.com -c user.name=Test \
            -c commit.gpgsign=false commit -qm 'unmerged worker commit'
    fi

    (cd "$repo" && LEAD_CACHE_POOL="$pool" LEAD_RECLAIM_FORCE="$force" "$RECLAIM" a > /dev/null 2>&1)
    assert "$label: reclaim succeeds" test "$?" -eq 0
    assert "$label: the temporary directory is removed" test ! -e "$tmp_dir"
    assert "$label: the leftover is absent from the cache pool" \
        test -z "$(find "$pool" -type f -name worker-leftover -print -quit 2>/dev/null)"
}

check_worker_tmp_reclaim normal 0
check_worker_tmp_reclaim force 1

echo "passed: $PASS, failed: $FAIL"
[[ "$FAIL" -eq 0 ]]
