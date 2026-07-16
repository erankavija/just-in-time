#!/usr/bin/env bash
# jit-project-lead utility: apply safe, rule-based corrections to every
# MECHANICAL content-standards finding emitted by standards-scan.sh.
#
# Consumes the scanner's JSONL output (one classified finding per line) and,
# for each finding classified `mechanical` whose rule has a single unambiguous
# correction, rewrites the source issue so the flagged violation no longer
# holds. Judgment findings, and any issue/document with no mechanical finding,
# are left byte-for-byte untouched.
#
# Reference: references/standards-fix.md (in this skill) — the mechanical
# rule -> correction mapping, the excluded rules and why, and the contract.
#
# Storage boundary:
#   Every issue mutation is routed through the jit CLI (`jit issue show` to
#   read the current title/description, `jit issue update` to write the
#   correction). We never parse or write .jit/issues/*.json directly, so
#   issue writes preserve the CLI's configured event and atomicity contracts.
#
#   No mechanical rule in the scanner targets a document — every document
#   finding is `judgment` (heading/criterion/title/label rules are all
#   issue-only; the content rules that fire on documents are all judgment).
#   The fixer therefore never mutates a document. Were a mechanical document
#   rule ever added, its writer must use the repository's configured atomic
#   publication contract; see the reference doc.
#
# Usage:
#   .agents/skills/jit-project-lead/scripts/standards-fix.sh [<project-root>] [--findings <file>] [--dry-run]
#     project-root defaults to the current directory; must contain .jit/.
#     --findings <file>  consume this scanner-output file instead of running
#                        the scanner (also accepted on stdin if piped).
#     --dry-run          report the corrections without writing them.
#
# Output:
#   stdout — one JSON object per applied or skipped correction (JSONL):
#     target_kind "issue" | "document"
#     target      issue short-id (8 hex) | document path
#     rule        the corrected rule id
#     line        1-based source line, or 0 for whole-item corrections
#     action      "applied" | "dry-run" | "skipped"
#     detail      what was done, or why it was skipped
#   stderr — a one-line summary (counts).
#
# Determinism / idempotence:
#   Corrections are a pure function of the findings and the current content.
#   Re-running the fixer after it has run once produces no second-round change
#   for an already-fixed item (the scanner reports no finding, so nothing is
#   selected). Fresh REQ-NN ids are allocated deterministically in line order.
#
# Exit codes:
#   0 — completed (corrections, if any, reported on stdout)
#   2 — bad invocation (.jit/ missing; python3/base64/gawk/jit unavailable; scan failed)

set -euo pipefail

root="."
findings_file=""
dry_run=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --findings)
            findings_file="$2"
            shift 2
            ;;
        --dry-run)
            dry_run=1
            shift
            ;;
        --*)
            echo "ERROR: unknown option '$1'." >&2
            exit 2
            ;;
        *)
            root="$1"
            shift
            ;;
    esac
done

for tool in python3 base64 gawk jit; do
    if ! command -v "$tool" > /dev/null 2>&1; then
        echo "ERROR: required tool '$tool' not found on PATH." >&2
        exit 2
    fi
done

if [[ ! -d "$root/.jit" ]]; then
    echo "ERROR: $root/.jit not found. Run from a jit project root or pass one." >&2
    exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
scanner="$script_dir/standards-scan.sh"

# --- Obtain findings ------------------------------------------------------
# Precedence: explicit --findings file, then a pipe on stdin, else run the
# scanner against the project root ourselves.
findings="$(mktemp)"
mech="$(mktemp)"
recfile="$(mktemp)"
skipfile="$(mktemp)"
trap 'rm -f "$findings" "$mech" "$recfile" "$skipfile"' EXIT

if [[ -n "$findings_file" ]]; then
    cat "$findings_file" > "$findings"
elif [[ ! -t 0 ]]; then
    cat > "$findings"
else
    if ! "$scanner" "$root" > "$findings" 2> /dev/null; then
        echo "ERROR: scanner failed against $root." >&2
        exit 2
    fi
fi

# Keep only mechanical findings; everything else is left untouched by design.
python3 -c '
import json, sys
with open(sys.argv[1], encoding="utf-8") as stream:
    for line in stream:
        if not line.strip():
            continue
        record = json.loads(line)
        if record.get("classification") == "mechanical":
            print(json.dumps(record, separators=(",", ":"), ensure_ascii=False))
' "$findings" > "$mech"

# --- gawk description transform -------------------------------------------
# Applies the body-level mechanical corrections (heading level, criterion
# marker/id, anti-pattern section removal, missing Success Criteria) to one
# issue description. Driven by DIRECTIVES (rule|line pairs the scanner
# reported for this issue) so it corrects exactly what was flagged.
#
#   stdin   — the current description body
#   DIRECTIVES — "RULE|LINE;RULE|LINE;..." for body rules on this issue
#   SC_MISSING — 1 to append a `## Success Criteria` heading, else 0
#   RECFILE — path; one "rule<TAB>line<TAB>detail" record per applied fix
#   SKIPFILE — path; one "rule<TAB>line<TAB>detail" record per criterion the
#              fixer left unchanged because it could not be corrected (the
#              pathological all-REQ-NN-ids-reserved case)
#   stdout  — the corrected description body
read -r -d '' AWK_FIX <<'AWK' || true
function fmt(n) { return sprintf("REQ-%02d", n) }
function record(rule, ln, detail) {
    gsub(/\t/, " ", detail)
    printf "%s\t%s\t%s\n", rule, ln, detail >> RECFILE
}
function record_skip(rule, ln, detail) {
    gsub(/\t/, " ", detail)
    printf "%s\t%s\t%s\n", rule, ln, detail >> SKIPFILE
}
# Split a criterion line into its bullet prefix ("- " plus an optional
# GitHub checkbox) and the remaining criterion text. Mirrors the scanner's
# stripping so the rewrite lands where the scanner reads the marker.
function split_bullet(line, out,   rest, pfx) {
    rest = line
    match(rest, /^-[[:space:]]+/)
    pfx = substr(rest, 1, RLENGTH)
    rest = substr(rest, RLENGTH + 1)
    if (match(rest, /^\[[ xX]\][[:space:]]*/)) {
        pfx = pfx substr(rest, 1, RLENGTH)
        rest = substr(rest, RLENGTH + 1)
    }
    out[1] = pfx
    out[2] = rest
}
BEGIN {
    n = split(DIRECTIVES, d, ";")
    for (i = 1; i <= n; i++) {
        if (d[i] == "") continue
        split(d[i], kv, "|")
        rule = kv[1]; ln = kv[2] + 0
        rules_at[ln] = rules_at[ln] (rules_at[ln] ? "," : "") rule
        if (rule == "STD-ANTIPATTERN-SECTION") anti[ln] = 1
        if (rule == "STD-CRIT-UNMARKED" || rule == "STD-CRIT-REQID") critfix[ln] = 1
    }
}
{ lines[NR] = $0 }
END {
    total = NR

    # Phase A: reserved REQ numbers (well-formed criteria in the SC section).
    in_sc = 0; fence = 0
    for (i = 1; i <= total; i++) {
        l = lines[i]
        if (l ~ /^(```|~~~)/) fence = !fence
        if (!fence && l ~ /^#{2,}[[:space:]]/) {
            h = tolower(l); sub(/^#+[[:space:]]*/, "", h); sub(/[[:space:]]+$/, "", h)
            if (h ~ /(success[[:space:]]+criteria|acceptance[[:space:]]+criteria|definition[[:space:]]+of[[:space:]]+done)$/ || h == "criteria")
                in_sc = 1
            else
                in_sc = 0
        }
        if (!fence && in_sc && l ~ /^-[[:space:]]/) {
            crit = l; sub(/^-[[:space:]]+/, "", crit); sub(/^\[[ xX]\][[:space:]]*/, "", crit)
            if (match(crit, /^\[(hard|aspirational)\][[:space:]]+REQ-([0-9][0-9]):/, cap))
                reserved[cap[2] + 0] = 1
        }
    }

    # Phase B: anti-pattern deletion ranges (heading line through the line
    # before the next heading, or EOF). Recorded once at the heading line.
    for (ln in anti) {
        ln = ln + 0  # for..in yields string keys; force numeric comparison
        if (ln < 1 || ln > total) continue
        del[ln] = 1
        record("STD-ANTIPATTERN-SECTION", ln, "removed DAG-duplicating section: " lines[ln])
        j = ln + 1
        while (j <= total && lines[j] !~ /^#{1,6}[[:space:]]/) { del[j] = 1; j++ }
    }

    # Phase C: allocate fresh two-digit REQ ids to criterion fixes, in line
    # order, skipping reserved numbers. Deterministic and collision-free.
    cnt = 0
    for (ln in critfix)
        if (!del[ln]) crit_lines[++cnt] = ln + 0
    if (cnt > 0) {
        asort(crit_lines)
        next_num = 1
        for (k = 1; k <= cnt; k++) {
            ln = crit_lines[k]
            while (reserved[next_num]) next_num++
            if (next_num > 99) { assign[ln] = -1; continue }  # no 2-digit id free
            assign[ln] = next_num
            reserved[next_num] = 1
            next_num++
        }
    }

    # Phase D: emit the corrected body.
    for (i = 1; i <= total; i++) {
        if (del[i]) continue
        l = lines[i]
        rl = rules_at[i]
        if (rl ~ /STD-HEADING-H1/) {
            sub(/^#[[:space:]]/, "## ", l)
            record("STD-HEADING-H1", i, "promoted H1 to H2")
        }
        if (rl ~ /STD-HEADING-DEEP/) {
            sub(/^#{4,}[[:space:]]/, "### ", l)
            record("STD-HEADING-DEEP", i, "clamped heading to H3")
        }
        if (rl ~ /STD-CRIT-UNMARKED/) {
            if (assign[i] == -1) {
                record_skip("STD-CRIT-UNMARKED", i, "no free REQ-NN id (01-99 all reserved); criterion left unchanged")
            } else {
                split_bullet(l, parts)
                l = parts[1] "[hard] " fmt(assign[i]) ": " parts[2]
                record("STD-CRIT-UNMARKED", i, "inserted [hard] " fmt(assign[i]) " marker")
            }
        }
        if (rl ~ /STD-CRIT-REQID/) {
            if (assign[i] == -1) {
                record_skip("STD-CRIT-REQID", i, "no free REQ-NN id (01-99 all reserved); criterion left unchanged")
            } else {
                split_bullet(l, parts)
                marker = "hard"
                if (match(parts[2], /^\[(hard|aspirational)\]/, mk)) marker = mk[1]
                stmt = parts[2]
                sub(/^\[(hard|aspirational)\][[:space:]]*(REQ-[0-9]*:?)?[[:space:]]*/, "", stmt)
                l = parts[1] "[" marker "] " fmt(assign[i]) ": " stmt
                record("STD-CRIT-REQID", i, "rewrote id to " fmt(assign[i]))
            }
        }
        print l
    }

    # Phase E: append a Success Criteria heading if the issue had none.
    if (SC_MISSING + 0 == 1) {
        print ""
        print "## Success Criteria"
        record("STD-SC-MISSING", 0, "appended ## Success Criteria heading")
    }
}
AWK

# --- Title strip (mechanical STD-TITLE-EMBEDDED-ID) -----------------------
# Removes a leading conventional-commit prefix, an embedded short-id/position
# code/ordinal prefix, and any `(jit:<hex>)` token — exactly the shapes the
# scanner flags. Prints the cleaned title; empty output signals "unsafe,
# leave unchanged".
strip_title() {
    printf '%s' "$1" | gawk '
    {
        t = $0
        changed = 1
        while (changed) {
            changed = 0
            if (match(t, /^[A-Za-z]+\([^)]*\):[[:space:]]*/)) { t = substr(t, RSTART + RLENGTH); changed = 1 }
            if (match(t, /[[:space:]]*\(jit:[0-9a-f]+\)/))     { t = substr(t, 1, RSTART - 1) substr(t, RSTART + RLENGTH); changed = 1 }
            # Hex short-id prefix with a colon terminator (optionally a
            # `/segment` before it): `abc1234:`, `abc1234/S0:` -> strip through
            # the colon. Checked before the bare-slash form so `abc1234/S0: x`
            # loses the whole `abc1234/S0: `, not just `abc1234/`.
            if (match(t, /^[0-9a-fA-F]{6,}(\/[^:[:space:]]*)?:[[:space:]]*/)) { t = substr(t, RSTART + RLENGTH); changed = 1 }
            # Hex short-id prefix followed by a slash but no colon (`abc1234/foo`)
            # — the scanner flags `^[0-9a-fA-F]{6,}[/:]`, so this shape must also
            # be stripped or the finding survives the re-scan.
            if (match(t, /^[0-9a-fA-F]{6,}\//))               { t = substr(t, RSTART + RLENGTH); changed = 1 }
            if (match(t, /^[A-Za-z][0-9]+\/[A-Za-z]?[0-9]*:[[:space:]]*/))    { t = substr(t, RSTART + RLENGTH); changed = 1 }
            if (match(t, /^[0-9]+[.):][[:space:]]+/))          { t = substr(t, RSTART + RLENGTH); changed = 1 }
        }
        sub(/^[[:space:]]+/, "", t); sub(/[[:space:]]+$/, "", t)
        print t
    }'
}

# --- Record emitter -------------------------------------------------------
emit_record() { # kind target rule line action detail
    python3 -c '
import json, sys
print(json.dumps({
    "target_kind": sys.argv[1],
    "target": sys.argv[2],
    "rule": sys.argv[3],
    "line": int(sys.argv[4]),
    "action": sys.argv[5],
    "detail": sys.argv[6],
}, separators=(",", ":"), ensure_ascii=False))
' "$1" "$2" "$3" "$4" "$5" "$6"
}

action_word() { [[ "$dry_run" -eq 1 ]] && echo "dry-run" || echo "applied"; }

jsonl_has_rule() { # file kind target rule
    python3 -c '
import json, sys
with open(sys.argv[1], encoding="utf-8") as stream:
    found = any(
        (record := json.loads(line)).get("target_kind") == sys.argv[2]
        and record.get("target") == sys.argv[3]
        and record.get("rule") == sys.argv[4]
        for line in stream if line.strip()
    )
raise SystemExit(0 if found else 1)
' "$1" "$2" "$3" "$4"
}

jsonl_directives() { # file target
    python3 -c '
import json, sys
rules = {
    "STD-HEADING-H1", "STD-HEADING-DEEP", "STD-CRIT-UNMARKED",
    "STD-CRIT-REQID", "STD-ANTIPATTERN-SECTION",
}
values = []
with open(sys.argv[1], encoding="utf-8") as stream:
    for line in stream:
        if not line.strip():
            continue
        record = json.loads(line)
        if (
            record.get("target_kind") == "issue"
            and record.get("target") == sys.argv[2]
            and record.get("rule") in rules
        ):
            values.append("{}|{}".format(record["rule"], record["line"]))
print(";".join(values))
' "$1" "$2"
}

jsonl_targets() { # file kind
    python3 -c '
import json, sys
values = set()
with open(sys.argv[1], encoding="utf-8") as stream:
    for line in stream:
        if not line.strip():
            continue
        record = json.loads(line)
        if record.get("target_kind") == sys.argv[2]:
            values.add(record.get("target", ""))
for value in sorted(values):
    if value:
        print(value)
' "$1" "$2"
}

jsonl_document_rules() { # file target
    python3 -c '
import json, sys
with open(sys.argv[1], encoding="utf-8") as stream:
    for line in stream:
        if not line.strip():
            continue
        record = json.loads(line)
        if record.get("target_kind") == "document" and record.get("target") == sys.argv[2]:
            print("{}\t{}".format(record.get("rule", ""), record.get("line", 0)))
' "$1" "$2"
}

issues_changed=0
fixes_applied=0
fixes_skipped=0

# --- Issue pass -----------------------------------------------------------
while IFS= read -r sid; do
    [[ -z "$sid" ]] && continue

    # STD-LABEL-SLUG is a judgment finding (an 8-hex short id yields no
    # meaningful kebab slug, so a human chooses the bucket name); the mechanical
    # filter above already dropped it, so the fixer never sees or touches it.

    # Whole-item: embedded-id title.
    has_title_finding=0
    if jsonl_has_rule "$mech" issue "$sid" STD-TITLE-EMBEDDED-ID; then
        has_title_finding=1
    fi
    new_title=""
    change_title=0
    if [[ "$has_title_finding" -eq 1 ]]; then
        cur_title="$(cd "$root" && jit issue show "$sid" --field title 2> /dev/null)"
        cleaned="$(strip_title "$cur_title")"
        if [[ -z "$cleaned" || "$cleaned" == "$cur_title" ]]; then
            emit_record issue "$sid" STD-TITLE-EMBEDDED-ID 0 skipped \
                "stripping the embedded metadata would leave no title; left unchanged"
            fixes_skipped=$((fixes_skipped + 1))
        else
            new_title="$cleaned"
            change_title=1
        fi
    fi

    # Body-level directives + missing Success Criteria.
    directives="$(jsonl_directives "$mech" "$sid")"
    sc_missing=0
    if jsonl_has_rule "$mech" issue "$sid" STD-SC-MISSING; then
        sc_missing=1
    fi

    new_desc=""
    change_desc=0
    : > "$recfile"
    : > "$skipfile"
    if [[ -n "$directives" || "$sc_missing" -eq 1 ]]; then
        cur_desc="$(cd "$root" && jit issue show "$sid" --field description 2> /dev/null)"
        new_desc="$(printf '%s' "$cur_desc" | gawk \
            -v DIRECTIVES="$directives" -v SC_MISSING="$sc_missing" \
            -v RECFILE="$recfile" -v SKIPFILE="$skipfile" "$AWK_FIX")"
        [[ "$new_desc" != "$cur_desc" ]] && change_desc=1
    fi

    # Apply the mutation through the CLI (one update per issue).
    if [[ "$change_title" -eq 1 || "$change_desc" -eq 1 ]]; then
        if [[ "$dry_run" -eq 0 ]]; then
            args=("$sid")
            [[ "$change_title" -eq 1 ]] && args+=(-t "$new_title")
            [[ "$change_desc" -eq 1 ]] && args+=(-d "$new_desc")
            (cd "$root" && jit issue update "${args[@]}" --force -q) > /dev/null 2>&1
        fi
        issues_changed=$((issues_changed + 1))
    fi

    # Records: title first, then the gawk body records.
    if [[ "$change_title" -eq 1 ]]; then
        emit_record issue "$sid" STD-TITLE-EMBEDDED-ID 0 "$(action_word)" "cleaned title to: $new_title"
        fixes_applied=$((fixes_applied + 1))
    fi
    while IFS=$'\t' read -r rrule rline rdetail; do
        [[ -z "$rrule" ]] && continue
        emit_record issue "$sid" "$rrule" "$rline" "$(action_word)" "$rdetail"
        fixes_applied=$((fixes_applied + 1))
    done < "$recfile"

    # Skips: a mechanical correction that could not be applied (e.g. no free
    # REQ-NN id). Reported so nothing is silently left unfixed (REQ-04).
    while IFS=$'\t' read -r srule sline sdetail; do
        [[ -z "$srule" ]] && continue
        emit_record issue "$sid" "$srule" "$sline" skipped "$sdetail"
        fixes_skipped=$((fixes_skipped + 1))
    done < "$skipfile"
done < <(jsonl_targets "$mech" issue)

# --- Document pass --------------------------------------------------------
# No mechanical rule targets documents; any mechanical document finding is
# unsupported and reported as skipped rather than silently applied.
while IFS= read -r dpath; do
    [[ -z "$dpath" ]] && continue
    while IFS=$'\t' read -r drule dline; do
        [[ -z "$drule" ]] && continue
        emit_record document "$dpath" "$drule" "$dline" skipped \
            "no mechanical correction defined for a document target"
        fixes_skipped=$((fixes_skipped + 1))
    done < <(jsonl_document_rules "$mech" "$dpath")
done < <(jsonl_targets "$mech" document)

if [[ "$dry_run" -eq 1 ]]; then
    echo "[fix] dry-run: ${issues_changed} issues would change, ${fixes_applied} corrections, ${fixes_skipped} skipped" >&2
else
    echo "[fix] ${issues_changed} issues changed, ${fixes_applied} corrections applied, ${fixes_skipped} skipped" >&2
fi
