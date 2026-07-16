#!/usr/bin/env bash
# jit-project-lead utility: scan every project issue and every in-scope
# document against the canonical content standards
# (.jit/reference/content-standards.md) and emit one classified finding
# per violation.
#
# Deterministic by construction: inputs are read in a fixed order, no
# timestamps or randomness enter the output, and findings are totally
# ordered before printing. Running it twice against an unchanged project
# yields byte-identical stdout.
#
# Reference: references/standards-scan.md (in this skill) — rule catalog,
# classification rationale, scope rules, output schema.
#
# Consumed by: the mechanical auto-fixer (mechanical findings) and the
# sweep report/workflow (all findings).
#
# Usage:
#   .agents/skills/jit-project-lead/scripts/standards-scan.sh [<project-root>]
#     project-root defaults to the current directory. Must contain .jit/.
#
# Output:
#   stdout — one JSON object per line (JSONL), fields:
#     target_kind    "issue" | "document"
#     target         issue short-id (8 hex) | document path (repo-relative)
#     rule           rule id, e.g. STD-CRIT-UNMARKED (see reference doc)
#     classification "mechanical" | "judgment"
#     line           1-based line in the description/document; 0 for
#                    whole-item rules (title, labels, missing section)
#     detail         offending text (tabs stripped), for locating the hit
#   stderr — a one-line summary (counts); never mixed into stdout.
#
# Storage boundary:
#   Issue title/description/labels/state and document linkage are read only
#   through the jit CLI (`jit issue list --full --json`) — never by parsing
#   .jit/issues/*.json directly. The CLI is the sanctioned access path to
#   persisted issue state. Only .jit/config.toml (configuration, not issue
#   persistence) is read as a file, since no CLI surface exposes the
#   [documentation] scope.
#
# Exit codes:
#   0 — scan completed (findings, if any, on stdout)
#   2 — bad invocation (.jit/ missing, python3/base64/gawk/jit unavailable, or the CLI
#       issue listing failed)

set -euo pipefail

root="${1:-.}"

for tool in python3 base64 gawk jit; do
    if ! command -v "$tool" > /dev/null 2>&1; then
        echo "ERROR: required tool '$tool' not found on PATH." >&2
        exit 2
    fi
done

jitroot="$root/.jit"
if [[ ! -d "$jitroot" ]]; then
    echo "ERROR: $jitroot not found. Run from a jit project root or pass one." >&2
    exit 2
fi

config="$jitroot/config.toml"

# --- Scope from [documentation] config (project-agnostic) -----------------
# permanent_paths: always in scope (e.g. docs/). development_root/active: the
# live development area. Other managed paths (studies, sessions) and the
# archive root are historical record and out of scope (see reference doc).
permanent_paths=()
if [[ -f "$config" ]]; then
    while IFS= read -r p; do
        [[ -n "$p" ]] && permanent_paths+=("$p")
    done < <(gawk '
        /^[[:space:]]*permanent_paths[[:space:]]*=/ {
            while (match($0, /"[^"]*"/)) {
                s = substr($0, RSTART + 1, RLENGTH - 2)
                print s
                $0 = substr($0, RSTART + RLENGTH)
            }
        }' "$config")
fi
[[ ${#permanent_paths[@]} -eq 0 ]] && permanent_paths=("docs/")

dev_root="$(gawk -F'"' '/^[[:space:]]*development_root[[:space:]]*=/ { print $2; exit }' "$config" 2>/dev/null || true)"
[[ -z "$dev_root" ]] && dev_root="dev"
active_dir="$root/$dev_root/active"

# --- Strategic membership-label namespaces (project-agnostic) --------------
# STD-LABEL-SLUG checks the strategic membership labels. Their namespaces are
# the values under [type_hierarchy.label_associations] in config — derived
# here, never hardcoded, so the rule tracks whatever tiers a project declares.
# A project with no associations declared simply has no strategic labels to
# slug-check and the rule never fires.
declare -A strategic_ns
if [[ -f "$config" ]]; then
    while IFS= read -r nsval; do
        [[ -n "$nsval" ]] && strategic_ns["$nsval"]=1
    done < <(gawk '
        /^[[:space:]]*\[/ { in_sec = (index($0, "[type_hierarchy.label_associations]") > 0) }
        in_sec && /=/ {
            if (match($0, /"[^"]*"/)) {
                print substr($0, RSTART + 1, RLENGTH - 2)
            }
        }' "$config")
fi

# --- Load every issue through the jit CLI (sanctioned storage path) --------
# `jit issue list --full --json` returns each issue's title, description,
# labels, state, and linked documents in a single call. We never parse
# .jit/issues/*.json directly. Cached in a temp file and reused; the embedded
# Python JSON reader sorts by id so output order is byte-stable across runs.
raw="$(mktemp)"
issues_json="$(mktemp)"
trap 'rm -f "$raw" "$issues_json"' EXIT

if ! (cd "$root" && jit issue list --full --json) > "$issues_json" 2>/dev/null; then
    echo "ERROR: 'jit issue list --full --json' failed in $root." >&2
    exit 2
fi

# --- Ownership maps for the dev/active exemption --------------------------
# A dev/active doc is live unless its owning issue is Done. Ownership is by
# the doc's <short-id>- filename prefix or its jit doc linkage (documents[]).
declare -A issue_state    # short_id -> state
declare -A docpath_state  # repo-relative doc path -> owning issue state

while IFS=$'\t' read -r sid st; do
    [[ -n "$sid" ]] && issue_state["$sid"]="$st"
done < <(python3 -c '
import json, sys
for issue in json.load(open(sys.argv[1], encoding="utf-8")).get("issues", []):
    print("{}\t{}".format(issue.get("id", "")[:8], issue.get("state", "")))
' "$issues_json")
while IFS=$'\t' read -r dp st; do
    [[ -n "$dp" ]] && docpath_state["$dp"]="$st"
done < <(python3 -c '
import json, sys
for issue in json.load(open(sys.argv[1], encoding="utf-8")).get("issues", []):
    for document in issue.get("documents") or []:
        print("{}\t{}".format(document.get("path", ""), issue.get("state", "")))
' "$issues_json")

# is_live_active_doc <repo-relative-path> <basename> -> 0 live, 1 exempt
is_live_active_doc() {
    local rel="$1" base="$2" prefix owner
    if [[ "$base" =~ ^([0-9a-f]{8})- ]]; then
        prefix="${BASH_REMATCH[1]}"
        owner="${issue_state[$prefix]:-}"
        [[ "$owner" == "done" ]] && return 1
    fi
    owner="${docpath_state[$rel]:-}"
    [[ "$owner" == "done" ]] && return 1
    return 0
}

# --- Finding emitter (raw TSV; JSON built + sorted at the end) -------------
emit() {
    # kind, target, rule, class, line, detail
    local detail="$6"
    detail="${detail//$'\t'/ }"
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" "$detail" >> "$raw"
}

# --- awk pass over a description or document body --------------------------
# Emits TSV findings for structure/heading/criterion/content rules. Issue-only
# rules (cross-ref, standalone, tracker-mechanics, missing-SC) run when
# kind==issue. own = this issue's short id (excluded from short-id refs).
read -r -d '' AWK_BODY <<'AWK' || true
function emit(rule, cls, ln, detail) {
    gsub(/\t/, " ", detail)
    printf "%s\t%s\t%s\t%s\t%s\t%s\n", tk, target, rule, cls, ln, detail
}
BEGIN { fence = 0; mfence = 0; in_sc = 0; sc_seen = 0; first_seen = 0
        summary_prose = 0; first_heading = 0; bg_seen = 0; pre_sc = 0 }
{
    line = $0
    ln = NR
    if (line ~ /^(```|~~~)/) fence = !fence

    # Display-math delimiter: `$$` alone on a line opens/closes a math region.
    # The delimiter line itself is neither prose nor math content; toggling
    # here means the lines strictly between a pair carry mfence == 1.
    is_mdelim = (line ~ /^[[:space:]]*\$\$[[:space:]]*$/)
    if (is_mdelim) mfence = !mfence

    # First content line: bare-pronoun opening (issue standalone heuristic).
    if (tk == "issue" && !first_seen && line !~ /^[[:space:]]*$/ && line !~ /^#/) {
        first_seen = 1
        if (line ~ /^(It|This|That|They|These|Those|Such)[[:space:]]/)
            emit("STD-STANDALONE", "judgment", ln, line)
    }

    if (!fence && !mfence && !is_mdelim) {
        # Heading-level rules (issue descriptions use ## / ###; never # or ####+).
        if (tk == "issue") {
            if (line ~ /^#[[:space:]]/)   emit("STD-HEADING-H1", "mechanical", ln, line)
            if (line ~ /^#{4,}[[:space:]]/) emit("STD-HEADING-DEEP", "mechanical", ln, line)
        }
        # Required-structure tracking (evaluated in END): a leading summary
        # must precede the first heading, and non-trivial issues carry a
        # `## Background`. A blank/heading line is neither summary nor context
        # prose; bullets are excluded from the context count.
        if (tk == "issue") {
            _blank = (line ~ /^[[:space:]]*$/)
            _head  = (line ~ /^#/)
            if (!first_heading && _head) first_heading = 1
            if (!first_heading && !_blank && !_head) summary_prose = 1
            if (_head) {
                bh = tolower(line); sub(/^#+[[:space:]]*/, "", bh); sub(/[[:space:]]+$/, "", bh)
                if (bh == "background") bg_seen = 1
            }
            # Context prose preceding the Success Criteria section. A large
            # such region without a `## Background` heading is the
            # non-trivial-issue signal that context was left unstructured.
            if (!sc_seen && !_blank && !_head && line !~ /^[[:space:]]*[-*+][[:space:]]/)
                pre_sc++
        }
        # Anti-pattern sections that duplicate the DAG (issue descriptions only;
        # standalone documents may legitimately carry such headings).
        if (tk == "issue" && line ~ /^#{2,}[[:space:]]*([Dd]epends[[:space:]]+[Oo]n|[Dd]ependencies|[Cc]hildren)[[:space:]]*$/)
            emit("STD-ANTIPATTERN-SECTION", "mechanical", ln, line)

        # Success Criteria section tracking (case-tolerant + legacy variants).
        if (line ~ /^#{2,}[[:space:]]/) {
            h = tolower(line)
            sub(/^#+[[:space:]]*/, "", h)
            sub(/[[:space:]]+$/, "", h)
            if (h ~ /(success[[:space:]]+criteria|acceptance[[:space:]]+criteria|definition[[:space:]]+of[[:space:]]+done)$/ || h == "criteria") {
                sc_seen = 1; in_sc = 1
            } else {
                in_sc = 0
            }
        }

        # Criterion marker rules, only for top-level bullets inside the section.
        # Strip an optional GitHub checkbox first so "- [ ] [hard] REQ-01: x"
        # is judged on its criticality marker, not the checkbox. A well-formed
        # id is `[hard]`/`[aspirational]` + `REQ-NN` (exactly two digits) + `:`.
        if (tk == "issue" && in_sc && line ~ /^-[[:space:]]/) {
            crit = line
            sub(/^-[[:space:]]+/, "", crit)
            sub(/^\[[ xX]\][[:space:]]*/, "", crit)
            stmt = ""
            if (crit ~ /^\[(hard|aspirational)\][[:space:]]+REQ-[0-9][0-9]:/) {
                stmt = crit
                sub(/^\[(hard|aspirational)\][[:space:]]+REQ-[0-9][0-9]:[[:space:]]*/, "", stmt)
            } else if (crit ~ /^\[(hard|aspirational)\]/) {
                # Marker present but the REQ-NN id is malformed (wrong digit
                # count, missing/misspelled REQ token, or missing colon).
                emit("STD-CRIT-REQID", "mechanical", ln, line)
                stmt = crit
                sub(/^\[(hard|aspirational)\][[:space:]]*(REQ-[0-9]*:?)?[[:space:]]*/, "", stmt)
            } else {
                emit("STD-CRIT-UNMARKED", "mechanical", ln, line)
                stmt = crit
            }
            # Criteria state observable outcomes, not actions (judgment).
            if (tolower(stmt) ~ /^(implement|add|create|build|write|refactor|rewrite|fix|remove|delete|update|rename|move|setup|integrate|configure)[[:space:]]/)
                emit("STD-CRIT-ACTION", "judgment", ln, line)
        }
    }

    # Content rules apply to issue descriptions and documents alike.
    if (line ~ /[┌┐└┘─│├┤┬┴┼╭╮╰╯║═╔╗╚╝]/)
        emit("STD-ASCII-ART", "judgment", ln, line)     # box-drawing glyphs
    else if (line ~ /\+[-=]{2,}\+/ || line ~ /\+[-=]{3,}/)
        emit("STD-ASCII-ART", "judgment", ln, line)     # +---+ ASCII box art
    if (line !~ /\$/ && (line ~ /(^|[[:space:]=(])(sum|prod|sqrt|frac)_/ || line ~ /[[:space:]]=[[:space:]]sum[[:space:]]/))
        emit("STD-PLAINTEXT-MATH", "judgment", ln, line)

    # LaTeX math-notation rules (issue descriptions and documents alike).
    if (!fence) {
        # Display fraction written with a slash instead of \frac{a}{b}.
        if (mfence && !is_mdelim && line ~ /[A-Za-z0-9})][[:space:]]*\/[[:space:]]*[A-Za-z0-9({\\]/)
            emit("STD-MATH-SLASH-FRAC", "judgment", ln, line)
        # Multi-letter sub/superscript label not wrapped in \text{...}.
        if ((mfence || line ~ /\$/) && !is_mdelim && line ~ /[_^]\{[A-Za-z][A-Za-z]/)
            emit("STD-MATH-MULTI-LABEL", "judgment", ln, line)
        # Inline math carrying display-style summation/integral limits.
        if (!mfence && line ~ /\$[^$]*\\(sum|int)_[^$]*\^[^$]*\$/)
            emit("STD-MATH-INLINE-LIMITS", "judgment", ln, line)
        # Bare variable names outside math mode ($x$ not x, $N_0$ not N_0).
        # Inline math spans are correct and stripped first; inline code is kept
        # so that a code-wrapped single letter still counts as a bare variable.
        if (!mfence && !is_mdelim) {
            prose = line
            gsub(/\$[^$]*\$/, " ", prose)
            mathvar = 0
            if (prose ~ /(^|[^A-Za-z0-9_])[A-Za-z0-9][_^][0-9A-Za-z]([^A-Za-z0-9_]|$)/)
                mathvar = 1
            else {
                p = prose
                while (match(p, /(^|[^A-Za-z0-9_])([A-Za-z])([^A-Za-z0-9_]|$)/, mm)) {
                    if (mm[2] != "a" && mm[2] != "A" && mm[2] != "I") { mathvar = 1; break }
                    p = substr(p, RSTART + RLENGTH - 1)
                    if (RLENGTH <= 1) break
                }
            }
            if (mathvar) emit("STD-MATH-BARE-VAR", "judgment", ln, line)
        }
    }

    # Issue-only prose heuristics.
    if (tk == "issue") {
        lc = tolower(line)
        if (lc ~ /(same as[[:space:]]|as (described|mentioned|noted|shown|above)|see above|the previous issue|previous item|sibling issue|per (a[0-9]|the sibling))/)
            emit("STD-CROSSREF", "judgment", ln, line)
        # Tracker mechanics: spelled-out jit commands, or naming/instructing the
        # gate or reviewer (pluggable enforcement must not leak into the issue).
        trk = 0
        if (line ~ /jit[[:space:]]+(issue|dep|gate|claim|doc|validate|query|graph|recover|init|apply|config)([[:space:]]|$)/)
            trk = 1
        else if (lc ~ /(gates?[[:space:]]+must[[:space:]]+pass|pass(es|ing)?[[:space:]]+the[[:space:]]+([a-z-]+[[:space:]]+)?gate|the[[:space:]]+reviewer|adversarial[[:space:]]+review|code-review[[:space:]]+gate)/)
            trk = 1
        if (trk) emit("STD-TRACKER-MECHANICS", "judgment", ln, line)
        # bare sibling short-id reference (8-hex containing a letter, not own id)
        s = line
        while (match(s, /(^|[^0-9a-zA-Z])([0-9a-f]{8})([^0-9a-zA-Z]|$)/, m)) {
            tok = m[2]
            if (tok != own && tok ~ /[a-f]/)
                emit("STD-STANDALONE", "judgment", ln, tok)
            s = substr(s, RSTART + RLENGTH - 1)
            if (RLENGTH <= 1) break
        }
    }
}
END {
    if (tk == "issue" && !sc_seen)
        emit("STD-SC-MISSING", "mechanical", 0, "issue has no Success Criteria heading")
    # Required structure: summary line before the first heading; `## Background`
    # for non-trivial issues (a substantial pre-criteria context region).
    if (tk == "issue" && !summary_prose)
        emit("STD-STRUCT-SUMMARY", "judgment", 0, "no summary line precedes the first heading")
    if (tk == "issue" && !bg_seen && pre_sc > 4)
        emit("STD-STRUCT-BACKGROUND", "judgment", 0, "non-trivial issue has no ## Background section")
}
AWK

scan_body() {
    # kind, target, own-id, body-text
    printf '%s' "$4" | gawk -v tk="$1" -v target="$2" -v own="$3" "$AWK_BODY" >> "$raw"
}

# --- Issue pass -----------------------------------------------------------
# Each row is one issue from the cached CLI listing, sorted by id. Python
# base64-encodes free-form fields so tabs and newlines survive the shell stream.
while IFS=$'\t' read -r sid title64 desc64 labels64; do
    title="$(printf '%s' "$title64" | base64 --decode)"
    desc="$(printf '%s' "$desc64" | base64 --decode)"

    # Title: embedded id / ordinal / conventional-commit prefix (mechanical).
    if [[ "$title" =~ ^[0-9a-fA-F]{6,}[/:] ]] \
        || [[ "$title" =~ \(jit:[0-9a-f]+\) ]] \
        || [[ "$title" =~ ^[A-Za-z]+\([^\)]*\): ]] \
        || [[ "$title" =~ ^[0-9]+[.\):][[:space:]] ]] \
        || [[ "$title" =~ ^[A-Za-z][0-9]+/[A-Za-z]?[0-9]*: ]]; then
        emit issue "$sid" STD-TITLE-EMBEDDED-ID mechanical 0 "$title"
    fi
    # Title: escaped angle brackets need rewording (judgment).
    if [[ "$title" == *"&lt;"* || "$title" == *"&gt;"* || "$title" == *"<"* || "$title" == *">"* ]]; then
        emit issue "$sid" STD-TITLE-ANGLE judgment 0 "$title"
    fi
    # Strategic labels must be kebab slugs, not the 8-hex short id (judgment).
    # No mechanical correction exists: a meaningful bucket slug (e.g. user-auth)
    # cannot be derived from a hash, so choosing one needs human judgment.
    while IFS= read -r lbl; do
        [[ -z "$lbl" ]] && continue
        ns="${lbl%%:*}"
        val="${lbl#*:}"
        if [[ -n "${strategic_ns[$ns]:-}" ]] && [[ "$val" =~ ^[0-9a-f]{8}$ ]]; then
            emit issue "$sid" STD-LABEL-SLUG judgment 0 "$lbl"
        fi
    done < <(printf '%s' "$labels64" | base64 --decode)

    scan_body issue "$sid" "$sid" "$desc"
done < <(python3 -c '
import base64, json, sys
def enc(value):
    return base64.b64encode(value.encode("utf-8")).decode("ascii")
issues = json.load(open(sys.argv[1], encoding="utf-8")).get("issues", [])
for issue in sorted(issues, key=lambda value: value.get("id", "")):
    labels = "\n".join(issue.get("labels") or [])
    print("\t".join([
        issue.get("id", "")[:8],
        enc(issue.get("title") or ""),
        enc(issue.get("description") or ""),
        enc(labels),
    ]))
' "$issues_json")

# --- Document pass --------------------------------------------------------
scan_doc_dir() {
    # dir, apply-active-exemption(0/1)
    local dir="$1" exempt="$2" md base rel
    [[ -d "$dir" ]] || return 0
    while IFS= read -r md; do
        rel="${md#"$root"/}"
        base="$(basename "$md")"
        if [[ "$exempt" == "1" ]]; then
            is_live_active_doc "$rel" "$base" || continue
        fi
        scan_body document "$rel" "" "$(cat "$md")"
    done < <(find "$dir" -type f -name '*.md' | LC_ALL=C sort)
}

for p in "${permanent_paths[@]}"; do
    scan_doc_dir "$root/${p%/}" 0
done
scan_doc_dir "$active_dir" 1

# --- Total-order the findings and print JSONL -----------------------------
python3 -c '
import json, sys
records = []
with open(sys.argv[1], encoding="utf-8") as stream:
    for raw in stream:
        fields = raw.rstrip("\n").split("\t", 5)
        if len(fields) != 6:
            continue
        records.append({
            "target_kind": fields[0],
            "target": fields[1],
            "rule": fields[2],
            "classification": fields[3],
            "line": int(fields[4]),
            "detail": fields[5],
        })
for record in sorted(records, key=lambda value: (
    value["target_kind"], value["target"], value["rule"],
    value["line"], value["detail"],
)):
    print(json.dumps(record, separators=(",", ":"), ensure_ascii=False))
' "$raw"

total="$(wc -l < "$raw" | tr -d ' ')"
mech="$(cut -f4 "$raw" | grep -c '^mechanical$' || true)"
judg="$(cut -f4 "$raw" | grep -c '^judgment$' || true)"
echo "[scan] ${total} findings: ${mech} mechanical, ${judg} judgment" >&2
