#!/usr/bin/env bash
# Test suite for standards-scan.sh.
#
# Builds a self-contained jit fixture project (temp dir, `jit init`), seeds
# issues and documents that exercise every asserted behavior, runs the
# scanner, and checks the emitted findings. Covers:
#   - mechanical criterion cases, including the malformed-REQ-NN variants
#     (one digit, three digits, missing colon, missing REQ token);
#   - a well-formed `[hard] REQ-NN:` criterion producing no finding;
#   - other mechanical cases: embedded-id title, missing Success Criteria;
#   - the judgment standalone case (bare-pronoun opening);
#   - the math rules the scanner must evaluate (a/b in display math, bare
#     variable outside math mode);
#   - scope filtering: docs/ scanned, live dev/active scanned, dev/active
#     owned by a Done issue exempt, dev/archive and dev/studies out of scope;
#   - determinism: two runs on the unchanged fixture are byte-identical.
#
# Run:
#   .agents/skills/jit-project-lead/scripts/test-standards-scan.sh
# Exit 0 when all assertions pass; 1 otherwise. Requires jit, jq, gawk.

# Not `-e`: assertions inspect command output rather than aborting on it.
set -uo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; NC='\033[0m'
PASS=0; FAIL=0

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCANNER="$SCRIPT_DIR/standards-scan.sh"

for tool in jit jq gawk; do
    command -v "$tool" > /dev/null 2>&1 || { echo "ERROR: '$tool' not on PATH"; exit 2; }
done
[[ -x "$SCANNER" ]] || { echo "ERROR: scanner not found at $SCANNER"; exit 2; }

FIX="$(mktemp -d)"
FINDINGS="$FIX.findings.jsonl"
FINDINGS2="$FIX.findings2.jsonl"
# shellcheck disable=SC2329  # invoked via trap
cleanup() { rm -rf "$FIX" "$FINDINGS" "$FINDINGS2"; }
trap cleanup EXIT

# --- Fixture project ------------------------------------------------------
(
    cd "$FIX"
    git init -q
    git config user.email test@example.com
    git config user.name Test
    jit init > /dev/null 2>&1
    # Explicit documentation scope (docs/ permanent, dev/active live).
    cat >> .jit/config.toml <<'TOML'

[documentation]
development_root = "dev"
permanent_paths = ["docs/"]
archive_root = "dev/archive"
TOML
) || { echo "ERROR: fixture init failed"; exit 2; }

mk() { # title, description, extra-args... -> prints short_id
    (cd "$FIX" && jit issue create "$1" -d "$2" "${@:3}" --json 2>/dev/null) | jq -r '.short_id'
}

# 1. Clean issue: well-formed REQ-NN, no triggers -> expect zero findings.
GOOD="$(mk "Clean parser issue" $'The parser reads configuration files.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the parsed value for valid configuration.')"

# 2. Malformed REQ-NN variants (marker present, id malformed) -> REQID each.
MALF="$(mk "Malformed criteria issue" $'A summary of malformed criteria.\n\n## Success Criteria\n\n- [hard] REQ-1: one digit is malformed.\n- [hard] REQ-123: three digits is malformed.\n- [hard] REQ-02 missing the colon.\n- [hard] no req token at all.\n- [aspirational] REQ-03: this one is well formed.' --force)"

# 3. Unmarked criterion -> UNMARKED.
UNMARK="$(mk "Unmarked criteria issue" $'A summary here.\n\n## Success Criteria\n\n- Returns the value without any criticality marker.' --force)"

# 4. Embedded-id title + no Success Criteria -> TITLE-EMBEDDED-ID + SC-MISSING.
BADTITLE="$(mk "abc1234/S0: embedded id title" "A description with no success criteria section." --force)"

# 5. Bare-pronoun opening -> STANDALONE (judgment).
STANDALONE="$(mk "Depends on outer context" $'It cannot be understood without the parent spec.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the completed result.' --force)"

# 6. Missing leading summary (description opens with a heading) -> STRUCT-SUMMARY.
NOSUMMARY="$(mk "No summary issue" $'## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.' --force)"

# 7. Non-trivial issue with a substantial pre-criteria context region but no
#    `## Background` heading -> STRUCT-BACKGROUND. Six context prose lines.
NOBG="$(mk "Unstructured context issue" $'A summary sentence.\n\nThe subsystem has a long history.\nSeveral components interact in non-obvious ways.\nThe prior implementation had performance issues.\nA redesign was proposed in an earlier quarter.\nThat redesign informs the current approach here.\nOne more line of loose context to cross the bar.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.' --force)"

# 8. Non-trivial issue that DOES structure its context under `## Background`
#    -> no STRUCT-BACKGROUND even though the region is long.
HASBG="$(mk "Structured context issue" $'A summary sentence.\n\n## Background\n\nThe subsystem has a long history.\nSeveral components interact in non-obvious ways.\nThe prior implementation had performance issues.\nA redesign was proposed in an earlier quarter.\nThat redesign informs the current approach here.\nOne more line of loose context to cross the bar.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.' --force)"

# 9. LEADING position code (`S0/W1: ...`) -> TITLE-EMBEDDED-ID (mechanical). The
#    scanner's position-code branch is `^`-anchored, in lockstep with the fixer,
#    which strips only a leading position code.
LEADPOS="$(mk "S0/W1: build the worker pool" $'A summary line.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.' --force)"

# 10. MID-title position code (`Build S0/W1: worker`) -> NOT flagged. Per the
#     standard, STD-TITLE-EMBEDDED-ID is a *leading* short-id/ordinal/prefix; a
#     mid-title `S0/W1` is not that rule and the fixer cannot deterministically
#     strip it, so the scanner must not classify it mechanical.
MIDPOS="$(mk "Build S0/W1: worker" $'A summary line.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.' --force)"

# --- Documents ------------------------------------------------------------
# In-scope docs/ file with math violations (display a/b + bare var outside math).
mkdir -p "$FIX/docs"
cat > "$FIX/docs/math.md" <<'MD'
# Math notes

The ratio is written badly below.

$$
r = a/b + 1
$$

The variable x should be in math mode here.
MD

# Live dev/active doc (no owner) -> scanned. Bare var to make it detectable.
mkdir -p "$FIX/dev/active"
cat > "$FIX/dev/active/live-note.md" <<'MD'
# Live note

The quantity n grows without bound.
MD

# dev/active doc owned (filename prefix) by a Done issue -> exempt.
DONEOWNER="$(mk "Done owner issue" $'A finished issue.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the finished output.')"
(cd "$FIX" && jit issue update "$DONEOWNER" -s "done" > /dev/null 2>&1)
cat > "$FIX/dev/active/${DONEOWNER}-exempt.md" <<'MD'
# Exempt note

The variable z appears but this doc is exempt.
MD

# dev/active doc owned by a Done issue via doc linkage (no filename prefix).
LINKOWNER="$(mk "Link owner issue" $'Another finished issue.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the linked output.')"
cat > "$FIX/dev/active/linked-note.md" <<'MD'
# Linked note

The variable v appears but this doc is linked to a Done issue.
MD
(cd "$FIX" && jit doc add "$LINKOWNER" dev/active/linked-note.md --doc-type notes > /dev/null 2>&1)
(cd "$FIX" && jit issue update "$LINKOWNER" -s "done" > /dev/null 2>&1)

# Out-of-scope trees: dev/archive and dev/studies. Seed violations; expect none.
mkdir -p "$FIX/dev/archive" "$FIX/dev/studies"
printf '# Archived\n\nThe variable q is here but archived.\n' > "$FIX/dev/archive/old.md"
printf '# Study\n\nThe variable w is here but a study.\n' > "$FIX/dev/studies/study.md"

# --- Run scanner ----------------------------------------------------------
"$SCANNER" "$FIX" > "$FINDINGS" 2> /dev/null
SCAN_RC=$?

# --- Assertion helpers ----------------------------------------------------
# count RULE TARGET [DETAIL_SUBSTR]
count() {
    jq -c --arg r "$1" --arg t "$2" --arg d "${3:-}" \
        'select(.rule==$r and .target==$t and ($d=="" or ((.detail|tostring)|contains($d))))' \
        "$FINDINGS" 2>/dev/null | wc -l | tr -d ' '
}
pass() { echo -e "${GREEN}✓${NC} $1"; PASS=$((PASS + 1)); }
fail() { echo -e "${RED}✗${NC} $1"; FAIL=$((FAIL + 1)); }

assert_present() { # desc RULE TARGET [DETAIL]
    local n; n="$(count "$2" "$3" "${4:-}")"
    if [[ "$n" -ge 1 ]]; then pass "$1"; else fail "$1 (expected >=1 ${2} on ${3}${4:+ ~ }$4, got $n)"; fi
}
assert_absent() { # desc RULE TARGET [DETAIL]
    local n; n="$(count "$2" "$3" "${4:-}")"
    if [[ "$n" -eq 0 ]]; then pass "$1"; else fail "$1 (expected 0 ${2} on ${3}${4:+ ~ }$4, got $n)"; fi
}
target_count() { jq -c --arg t "$1" 'select(.target==$t)' "$FINDINGS" 2>/dev/null | wc -l | tr -d ' '; }

# --- Assertions -----------------------------------------------------------
if [[ "$SCAN_RC" -eq 0 ]]; then pass "scanner exits 0"; else fail "scanner exit code ($SCAN_RC)"; fi

# Mechanical: malformed REQ-NN variants each flagged (finding 2 regression).
assert_present "malformed REQ-1 (one digit) -> REQID"          STD-CRIT-REQID "$MALF" "REQ-1:"
assert_present "malformed REQ-123 (three digits) -> REQID"      STD-CRIT-REQID "$MALF" "REQ-123:"
assert_present "malformed REQ-02 missing colon -> REQID"        STD-CRIT-REQID "$MALF" "missing the colon"
assert_present "marker present but no REQ token -> REQID"       STD-CRIT-REQID "$MALF" "no req token"
# Well-formed REQ-NN must NOT be flagged, in the same issue or the clean one.
assert_absent  "well-formed REQ-03 not flagged (same issue)"   STD-CRIT-REQID "$MALF" "well formed"
if [[ "$(target_count "$GOOD")" -eq 0 ]]; then pass "clean issue (well-formed REQ-01) has zero findings"; else fail "clean issue has $(target_count "$GOOD") findings: $(jq -c --arg t "$GOOD" 'select(.target==$t)' "$FINDINGS")"; fi

# Other mechanical cases.
assert_present "unmarked criterion -> UNMARKED"                STD-CRIT-UNMARKED "$UNMARK"
assert_present "embedded-id title -> TITLE-EMBEDDED-ID"        STD-TITLE-EMBEDDED-ID "$BADTITLE"
assert_present "missing Success Criteria -> SC-MISSING"        STD-SC-MISSING "$BADTITLE"
# Position-code title branch is `^`-anchored (lockstep with the fixer's strip).
assert_present "leading position code -> TITLE-EMBEDDED-ID"    STD-TITLE-EMBEDDED-ID "$LEADPOS"
assert_absent  "mid-title position code NOT flagged"          STD-TITLE-EMBEDDED-ID "$MIDPOS"
if [[ "$(target_count "$MIDPOS")" -eq 0 ]]; then pass "mid-title position-code issue has zero findings"; else fail "mid-title position-code issue has $(target_count "$MIDPOS") findings: $(jq -c --arg t "$MIDPOS" 'select(.target==$t)' "$FINDINGS")"; fi

# Judgment: standalone bare-pronoun opening.
assert_present "bare-pronoun opening -> STANDALONE"            STD-STANDALONE "$STANDALONE" "It cannot be understood"

# Required-structure rules (required-structure coverage; finding 66aeee5f).
assert_present "no leading summary -> STRUCT-SUMMARY"          STD-STRUCT-SUMMARY "$NOSUMMARY"
assert_absent  "summary present -> no STRUCT-SUMMARY"          STD-STRUCT-SUMMARY "$GOOD"
assert_present "non-trivial, no Background -> STRUCT-BACKGROUND" STD-STRUCT-BACKGROUND "$NOBG"
assert_absent  "Background present -> no STRUCT-BACKGROUND"    STD-STRUCT-BACKGROUND "$HASBG"
assert_absent  "trivial leaf -> no STRUCT-BACKGROUND"         STD-STRUCT-BACKGROUND "$GOOD"

# Math rules the scanner must evaluate.
assert_present "a/b in display math -> MATH-SLASH-FRAC"        STD-MATH-SLASH-FRAC "docs/math.md" "a/b"
assert_present "bare variable outside math -> MATH-BARE-VAR"   STD-MATH-BARE-VAR   "docs/math.md" "variable x"

# Scope filtering.
if [[ "$(target_count "docs/math.md")" -ge 1 ]]; then pass "docs/ is in scope"; else fail "docs/math.md not scanned"; fi
assert_present "live dev/active doc scanned"                   STD-MATH-BARE-VAR "dev/active/live-note.md"
if [[ "$(target_count "dev/active/${DONEOWNER}-exempt.md")" -eq 0 ]]; then pass "dev/active owned-by-Done doc exempt (filename prefix)"; else fail "prefix-exempt doc was scanned"; fi
if [[ "$(target_count "dev/active/linked-note.md")" -eq 0 ]]; then pass "dev/active owned-by-Done doc exempt (doc linkage)"; else fail "linkage-exempt doc was scanned"; fi
if [[ "$(target_count "dev/archive/old.md")" -eq 0 ]]; then pass "dev/archive out of scope"; else fail "dev/archive scanned"; fi
if [[ "$(target_count "dev/studies/study.md")" -eq 0 ]]; then pass "dev/studies out of scope"; else fail "dev/studies scanned"; fi

# Determinism: a second run is byte-identical.
"$SCANNER" "$FIX" > "$FINDINGS2" 2> /dev/null
if diff -q "$FINDINGS" "$FINDINGS2" > /dev/null; then pass "two runs are byte-identical"; else fail "runs differ"; fi

# --- Summary --------------------------------------------------------------
echo
echo "========================================"
echo -e "${GREEN}Passed: $PASS${NC}"
if [[ "$FAIL" -gt 0 ]]; then
    echo -e "${RED}Failed: $FAIL${NC}"
    exit 1
fi
echo -e "${GREEN}All tests passed!${NC}"
exit 0
