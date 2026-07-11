#!/usr/bin/env bash
# Test suite for standards-fix.sh (the mechanical content-standards fixer).
#
# Builds a self-contained jit fixture (temp dir, `jit init`), seeds issues and
# documents that exercise every mechanical rule plus judgment/clean controls,
# runs the scanner to produce findings, runs the fixer, then re-scans and
# checks the four success criteria:
#
#   REQ-01  every fixable mechanical finding is corrected — the re-scan reports
#           none left, except a criterion the fixer genuinely cannot fix (all
#           REQ-NN ids reserved), which is reported skipped.
#   REQ-02  the re-scan no longer reports any of the fixed findings.
#   REQ-03  judgment findings (STD-LABEL-SLUG among them), clean issues, the
#           8-hex label, and in-scope documents are left byte-for-byte unchanged.
#   REQ-04  the fixer reports the issue and rule for every correction, and a
#           skip for any finding it could not fix.
#
#   plus determinism/idempotence: a second fixer run applies nothing.
#
# Run:
#   .agents/skills/jit-project-lead/scripts/test-standards-fix.sh
# Exit 0 when all assertions pass; 1 otherwise. Requires jit, jq, gawk.

set -uo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; NC='\033[0m'
PASS=0; FAIL=0

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCANNER="$SCRIPT_DIR/standards-scan.sh"
FIXER="$SCRIPT_DIR/standards-fix.sh"

for tool in jit jq gawk; do
    command -v "$tool" > /dev/null 2>&1 || { echo "ERROR: '$tool' not on PATH"; exit 2; }
done
[[ -x "$SCANNER" ]] || { echo "ERROR: scanner not found at $SCANNER"; exit 2; }
[[ -x "$FIXER" ]]   || { echo "ERROR: fixer not found at $FIXER"; exit 2; }

FIX="$(mktemp -d)"
BEFORE="$FIX.before.jsonl"
AFTER="$FIX.after.jsonl"
REPORT="$FIX.report.jsonl"
REPORT2="$FIX.report2.jsonl"
# shellcheck disable=SC2329  # invoked via trap
cleanup() { rm -rf "$FIX" "$BEFORE" "$AFTER" "$REPORT" "$REPORT2"; }
trap cleanup EXIT

# --- Fixture project ------------------------------------------------------
(
    cd "$FIX"
    git init -q
    git config user.email test@example.com
    git config user.name Test
    jit init > /dev/null 2>&1
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
desc_of() { (cd "$FIX" && jit issue show "$1" --field description 2>/dev/null); }
labels_of() { (cd "$FIX" && jit issue show "$1" --field labels 2>/dev/null); }

# 1. Clean issue: zero findings -> must be left untouched.
GOOD="$(mk "Clean parser issue" $'The parser reads configuration files.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the parsed value for valid configuration.')"

# 2. Malformed REQ-NN variants + one well-formed -> REQID fixes; keep REQ-03.
MALF="$(mk "Malformed criteria issue" $'A summary of malformed criteria.\n\n## Success Criteria\n\n- [hard] REQ-1: one digit is malformed.\n- [hard] REQ-123: three digits is malformed.\n- [hard] REQ-02 missing the colon.\n- [hard] no req token at all.\n- [aspirational] REQ-03: this one is well formed.' --force)"

# 3. Unmarked criterion -> UNMARKED fix.
UNMARK="$(mk "Unmarked criteria issue" $'A summary here.\n\n## Success Criteria\n\n- Returns the value without any criticality marker.' --force)"

# 4. Embedded-id title + no Success Criteria -> TITLE + SC-MISSING fixes.
BADTITLE="$(mk "abc1234/S0: embedded id title" "A description with no success criteria section." --force)"

# 5. Description heading levels: an H1 and a too-deep heading -> HEADING fixes.
HEADINGS="$(mk "Heading level issue" $'A summary line.\n\n# Wrong top-level heading\n\nSome context under it.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.\n\n#### Too deep heading\n\nTrailing prose.' --force)"

# 6. Anti-pattern section duplicating the DAG -> section removed.
ANTI="$(mk "Anti-pattern section issue" $'A summary line.\n\n## Depends on\n\n- some-parent-id\n- another-parent-id\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.' --force)"

# 7. Strategic label is an 8-hex short id -> LABEL-SLUG is EXCLUDED (skipped).
LABEL="$(mk "Label slug issue" $'A summary line.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.' -l epic:abc12345 --force)"

# 8. Judgment-only issue (bare-pronoun opening) -> nothing mechanical; untouched.
JUDGE="$(mk "Standalone readability issue" $'It cannot be understood without the parent spec.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the completed result.' --force)"

# 9. Mixed: a mechanical criterion fix must NOT disturb a co-located judgment
#    finding (the bare-pronoun opening line stays; only the criterion changes).
COMBO="$(mk "Mixed mechanical and judgment issue" $'It opens with a bare pronoun on purpose.\n\n## Success Criteria\n\n- Returns the value with no criticality marker at all.' --force)"

# 10. Embedded-id title as a hex prefix + bare slash, NO colon (`abc1234/foo`).
#     The scanner flags `^[0-9a-fA-F]{6,}[/:]`; the fixer must strip it too, or
#     the finding survives the re-scan (finding 2).
HEXSLASH="$(mk "abc1234/foo" $'A summary line.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.' --force)"

# 11. Exhausted REQ-NN ids: 99 well-formed criteria reserve 01-99, so the one
#     unmarked criterion has no free id. The fixer must leave it unchanged AND
#     report it skipped, not silently drop it (finding 3, REQ-04).
EXHAUST_SC=""
for i in $(seq 1 99); do
    EXHAUST_SC+="$(printf -- '- [hard] REQ-%02d: Returns result %d for a valid input.\n' "$i" "$i")"$'\n'
done
EXHAUST_DESC="A summary line.

## Success Criteria

${EXHAUST_SC}- An unmarked criterion that needs a fresh id but none is free."
EXHAUST="$(mk "Exhausted req ids issue" "$EXHAUST_DESC" --force)"

# 12. LEADING position-code title (`S0/W1: ...`). The scanner flags it mechanical
#     (branch is `^`-anchored); the fixer strips the leading position code, so it
#     must be gone after the re-scan and the title cleaned.
LEADPOS="$(mk "S0/W1: build the worker pool" $'A summary line.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.' --force)"

# 13. MID-title position code (`Build S0/W1: worker`). NOT a leading prefix, so
#     the scanner never flags it and the fixer never touches it — the title must
#     survive byte-for-byte (scanner/fixer contract: nothing flagged, nothing
#     changed, nothing left to re-flag).
MIDPOS="$(mk "Build S0/W1: worker" $'A summary line.\n\n## Success Criteria\n\n- [hard] REQ-01: Returns the value for a valid input.' --force)"
MIDPOS_TITLE_BEFORE="$(cd "$FIX" && jit issue show "$MIDPOS" --field title 2>/dev/null)"

# Documents: judgment-only math violations -> must be left byte-identical.
mkdir -p "$FIX/docs"
cat > "$FIX/docs/math.md" <<'MD'
# Math notes

The ratio is written badly below.

$$
r = a/b + 1
$$

The variable x should be in math mode here.
MD
DOC_SHA_BEFORE="$(sha256sum "$FIX/docs/math.md" | cut -d' ' -f1)"

# Capture pre-fix descriptions for the untouched-content assertions.
GOOD_DESC_BEFORE="$(desc_of "$GOOD")"
JUDGE_DESC_BEFORE="$(desc_of "$JUDGE")"
LABEL_LABELS_BEFORE="$(labels_of "$LABEL")"

# --- Run scanner, then fixer (consuming the scanner's output) --------------
"$SCANNER" "$FIX" > "$BEFORE" 2>/dev/null
"$FIXER" "$FIX" --findings "$BEFORE" > "$REPORT" 2>/dev/null
FIX_RC=$?
"$SCANNER" "$FIX" > "$AFTER" 2>/dev/null

# --- Assertion helpers ----------------------------------------------------
pass() { echo -e "${GREEN}✓${NC} $1"; PASS=$((PASS + 1)); }
fail() { echo -e "${RED}✗${NC} $1"; FAIL=$((FAIL + 1)); }

# scan_count RULE TARGET FILE -> number of matching findings
scan_count() { jq -c --arg r "$1" --arg t "$2" 'select(.rule==$r and .target==$t)' "$3" 2>/dev/null | wc -l | tr -d ' '; }
# report_count RULE TARGET ACTION -> matching applied/skipped records
report_count() { jq -c --arg r "$1" --arg t "$2" --arg a "$3" 'select(.rule==$r and .target==$t and .action==$a)' "$REPORT" 2>/dev/null | wc -l | tr -d ' '; }

assert_scan_absent() { # desc RULE TARGET FILE
    local n; n="$(scan_count "$2" "$3" "$4")"
    if [[ "$n" -eq 0 ]]; then pass "$1"; else fail "$1 (expected 0 ${2} on ${3}, got $n)"; fi
}
assert_scan_present() { # desc RULE TARGET FILE
    local n; n="$(scan_count "$2" "$3" "$4")"
    if [[ "$n" -ge 1 ]]; then pass "$1"; else fail "$1 (expected >=1 ${2} on ${3}, got $n)"; fi
}
assert_report() { # desc RULE TARGET ACTION [min]
    local n min; n="$(report_count "$2" "$3" "$4")"; min="${5:-1}"
    if [[ "$n" -ge "$min" ]]; then pass "$1"; else fail "$1 (expected >=${min} ${4} ${2} on ${3}, got $n)"; fi
}

# --- Assertions -----------------------------------------------------------
if [[ "$FIX_RC" -eq 0 ]]; then pass "fixer exits 0"; else fail "fixer exit code ($FIX_RC)"; fi

# REQ-01 / REQ-02: no *fixable* mechanical finding survives. STD-LABEL-SLUG is
# now judgment (a human chooses the bucket slug), so it never counts as
# mechanical. The exhausted-id issue's criterion is genuinely unfixable (all
# REQ-NN reserved) and is excluded here — its skip is asserted separately below.
LEFT="$(jq -c --arg e "$EXHAUST" 'select(.classification=="mechanical" and .target!=$e)' "$AFTER" 2>/dev/null | wc -l | tr -d ' ')"
if [[ "$LEFT" -eq 0 ]]; then pass "REQ-01/02: no fixable mechanical finding remains after fixer"; else fail "REQ-01/02: $LEFT fixable mechanical findings remain: $(jq -c --arg e "$EXHAUST" 'select(.classification=="mechanical" and .target!=$e)' "$AFTER")"; fi

# REQ-02: each specific fixed finding is gone.
assert_scan_present "sanity: REQID present before"            STD-CRIT-REQID "$MALF" "$BEFORE"
assert_scan_absent  "REQ-02: REQID gone after"                STD-CRIT-REQID "$MALF" "$AFTER"
assert_scan_absent  "REQ-02: no new UNMARKED on fixed MALF"   STD-CRIT-UNMARKED "$MALF" "$AFTER"
assert_scan_absent  "REQ-02: UNMARKED gone after"             STD-CRIT-UNMARKED "$UNMARK" "$AFTER"
assert_scan_absent  "REQ-02: TITLE-EMBEDDED-ID gone after"    STD-TITLE-EMBEDDED-ID "$BADTITLE" "$AFTER"
assert_scan_present "sanity: hex+slash title flagged before"  STD-TITLE-EMBEDDED-ID "$HEXSLASH" "$BEFORE"
assert_scan_absent  "REQ-02: hex+slash TITLE gone after"      STD-TITLE-EMBEDDED-ID "$HEXSLASH" "$AFTER"
assert_scan_present "sanity: leading position-code flagged before" STD-TITLE-EMBEDDED-ID "$LEADPOS" "$BEFORE"
assert_scan_absent  "REQ-02: leading position-code TITLE gone after" STD-TITLE-EMBEDDED-ID "$LEADPOS" "$AFTER"
assert_scan_absent  "mid-title position code never flagged"   STD-TITLE-EMBEDDED-ID "$MIDPOS" "$BEFORE"
assert_scan_absent  "REQ-02: SC-MISSING gone after"           STD-SC-MISSING "$BADTITLE" "$AFTER"
assert_scan_absent  "REQ-02: HEADING-H1 gone after"           STD-HEADING-H1 "$HEADINGS" "$AFTER"
assert_scan_absent  "REQ-02: HEADING-DEEP gone after"         STD-HEADING-DEEP "$HEADINGS" "$AFTER"
assert_scan_absent  "REQ-02: ANTIPATTERN-SECTION gone after"  STD-ANTIPATTERN-SECTION "$ANTI" "$AFTER"

# Well-formed REQ-03 in MALF survives (its number stays reserved, not reused).
if desc_of "$MALF" | grep -q "REQ-03: this one is well formed"; then pass "REQ-03 well-formed criterion preserved verbatim"; else fail "well-formed REQ-03 was altered"; fi

# REQ-04: the fixer records issue + rule for each correction.
assert_report "REQ-04: records UNMARKED fix"           STD-CRIT-UNMARKED "$UNMARK" applied
assert_report "REQ-04: records 4 REQID fixes"          STD-CRIT-REQID "$MALF" applied 4
assert_report "REQ-04: records TITLE fix"              STD-TITLE-EMBEDDED-ID "$BADTITLE" applied
assert_report "REQ-04: records SC-MISSING fix"        STD-SC-MISSING "$BADTITLE" applied
assert_report "REQ-04: records HEADING-H1 fix"        STD-HEADING-H1 "$HEADINGS" applied
assert_report "REQ-04: records HEADING-DEEP fix"      STD-HEADING-DEEP "$HEADINGS" applied
assert_report "REQ-04: records ANTIPATTERN fix"       STD-ANTIPATTERN-SECTION "$ANTI" applied

# Finding 3 / REQ-04: the exhausted-id criterion cannot be fixed (no free
# REQ-NN), so it is left unchanged AND reported skipped — never silently dropped.
assert_report      "REQ-04: records exhausted-id skip"        STD-CRIT-UNMARKED "$EXHAUST" skipped
assert_scan_present "exhausted-id criterion stays unmarked"   STD-CRIT-UNMARKED "$EXHAUST" "$AFTER"

# STD-LABEL-SLUG is now a judgment finding: the fixer never records it at all.
if [[ "$(report_count STD-LABEL-SLUG "$LABEL" skipped)" -eq 0 ]]; then pass "REQ-03: judgment LABEL-SLUG not recorded by fixer"; else fail "LABEL-SLUG still recorded as a fixer action"; fi

# Title actually cleaned.
if [[ "$(cd "$FIX" && jit issue show "$BADTITLE" --field title)" == "embedded id title" ]]; then pass "title stripped to clean form"; else fail "title not cleaned: $(cd "$FIX" && jit issue show "$BADTITLE" --field title)"; fi
if [[ "$(cd "$FIX" && jit issue show "$HEXSLASH" --field title)" == "foo" ]]; then pass "hex+slash title stripped to clean form"; else fail "hex+slash title not cleaned: $(cd "$FIX" && jit issue show "$HEXSLASH" --field title)"; fi
if [[ "$(cd "$FIX" && jit issue show "$LEADPOS" --field title)" == "build the worker pool" ]]; then pass "leading position-code title stripped to clean form"; else fail "leading position-code title not cleaned: $(cd "$FIX" && jit issue show "$LEADPOS" --field title)"; fi
if [[ "$(cd "$FIX" && jit issue show "$MIDPOS" --field title)" == "$MIDPOS_TITLE_BEFORE" ]]; then pass "REQ-03: mid-title position-code title left unchanged"; else fail "mid-title position-code title was altered: $(cd "$FIX" && jit issue show "$MIDPOS" --field title)"; fi

# REQ-03: clean issue untouched (byte-identical description).
if [[ "$(desc_of "$GOOD")" == "$GOOD_DESC_BEFORE" ]]; then pass "REQ-03: clean issue left unchanged"; else fail "REQ-03: clean issue description changed"; fi
GOOD_FINDINGS_AFTER="$(jq -c --arg t "$GOOD" 'select(.target==$t)' "$AFTER" | wc -l | tr -d ' ')"
if [[ "$GOOD_FINDINGS_AFTER" -eq 0 ]]; then pass "REQ-03: clean issue still reports zero findings"; else fail "clean issue now has $GOOD_FINDINGS_AFTER findings"; fi

# REQ-03: judgment-only issue untouched, and its judgment finding still fires.
if [[ "$(desc_of "$JUDGE")" == "$JUDGE_DESC_BEFORE" ]]; then pass "REQ-03: judgment-only issue left unchanged"; else fail "REQ-03: judgment-only issue changed"; fi
assert_scan_present "REQ-03: STANDALONE judgment still reported" STD-STANDALONE "$JUDGE" "$AFTER"

# REQ-03: mixed issue — criterion fixed, judgment pronoun line preserved.
assert_scan_absent  "REQ-03: mixed UNMARKED corrected"        STD-CRIT-UNMARKED "$COMBO" "$AFTER"
assert_scan_present "REQ-03: mixed STANDALONE untouched"      STD-STANDALONE "$COMBO" "$AFTER"
if desc_of "$COMBO" | grep -q "^It opens with a bare pronoun on purpose\.$"; then pass "REQ-03: mixed judgment line preserved verbatim"; else fail "mixed judgment opening line was altered"; fi

# REQ-03: judgment 8-hex label unchanged on the issue.
if [[ "$(labels_of "$LABEL")" == "$LABEL_LABELS_BEFORE" ]] && labels_of "$LABEL" | grep -q "epic:abc12345"; then pass "REQ-03: judgment 8-hex label left unchanged"; else fail "judgment label was modified: $(labels_of "$LABEL")"; fi

# REQ-03: in-scope document left byte-identical (all its findings are judgment).
if [[ "$(sha256sum "$FIX/docs/math.md" | cut -d' ' -f1)" == "$DOC_SHA_BEFORE" ]]; then pass "REQ-03: judgment-only document left byte-identical"; else fail "document was modified"; fi

# Idempotence: a second run applies nothing (only the excluded skip records).
"$FIXER" "$FIX" > "$REPORT2" 2>/dev/null
APPLIED2="$(jq -c 'select(.action=="applied")' "$REPORT2" 2>/dev/null | wc -l | tr -d ' ')"
if [[ "$APPLIED2" -eq 0 ]]; then pass "idempotent: second run applies no correction"; else fail "second run applied $APPLIED2 corrections"; fi

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
