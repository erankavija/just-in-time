#!/usr/bin/env bash
# jit-project-lead utility: scan every project issue and every in-scope
# document against the canonical content standards
# (docs/reference/jit-content-standards.md) and emit one classified finding
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
#   scripts/standards-scan.sh [<project-root>]
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
# Exit codes:
#   0 — scan completed (findings, if any, on stdout)
#   2 — bad invocation (.jit/ missing, or jq/gawk unavailable)

set -euo pipefail

root="${1:-.}"

for tool in jq gawk; do
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

# --- Ownership maps for the dev/active exemption --------------------------
# A dev/active doc is live unless its owning issue is Done. Ownership is by
# the doc's <short-id>- filename prefix or its jit doc linkage (documents[]).
declare -A issue_state    # short_id -> state
declare -A docpath_state  # repo-relative doc path -> owning issue state

shopt -s nullglob
issue_files=("$jitroot"/issues/*.json)
shopt -u nullglob

for f in "${issue_files[@]}"; do
    while IFS=$'\t' read -r sid st; do
        issue_state["$sid"]="$st"
    done < <(jq -r '[(.id[0:8]), .state] | @tsv' "$f")
    while IFS=$'\t' read -r dp st; do
        [[ -n "$dp" ]] && docpath_state["$dp"]="$st"
    done < <(jq -r '.state as $s | (.documents // [])[] | [.path, $s] | @tsv' "$f")
done

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
raw="$(mktemp)"
trap 'rm -f "$raw"' EXIT

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
BEGIN { fence = 0; in_sc = 0; sc_seen = 0; first_seen = 0 }
{
    line = $0
    ln = NR
    if (line ~ /^(```|~~~)/) fence = !fence

    # First content line: bare-pronoun opening (issue standalone heuristic).
    if (tk == "issue" && !first_seen && line !~ /^[[:space:]]*$/ && line !~ /^#/) {
        first_seen = 1
        if (line ~ /^(It|This|That|They|These|Those|Such)[[:space:]]/)
            emit("STD-STANDALONE", "judgment", ln, line)
    }

    if (!fence) {
        # Heading-level rules (issue descriptions use ## / ###; never # or ####+).
        if (tk == "issue") {
            if (line ~ /^#[[:space:]]/)   emit("STD-HEADING-H1", "mechanical", ln, line)
            if (line ~ /^#{4,}[[:space:]]/) emit("STD-HEADING-DEEP", "mechanical", ln, line)
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
        # Strip an optional GitHub checkbox first so "- [ ] [hard] REQ-1: x"
        # is judged on its criticality marker, not the checkbox.
        if (tk == "issue" && in_sc && line ~ /^-[[:space:]]/) {
            crit = line
            sub(/^-[[:space:]]+/, "", crit)
            sub(/^\[[ xX]\][[:space:]]*/, "", crit)
            if (crit ~ /^\[(hard|aspirational)\][[:space:]]+REQ-[0-9]+:/) {
                # well-formed
            } else if (crit ~ /^\[(hard|aspirational)\]/) {
                emit("STD-CRIT-REQID", "mechanical", ln, line)
            } else {
                emit("STD-CRIT-UNMARKED", "mechanical", ln, line)
            }
        }
    }

    # Content rules apply to issue descriptions and documents alike.
    if (line ~ /[┌┐└┘─│├┤┬┴┼╭╮╰╯║═╔╗╚╝]/)
        emit("STD-ASCII-ART", "judgment", ln, line)     # box-drawing glyphs
    else if (line ~ /\+[-=]{2,}\+/ || line ~ /\+[-=]{3,}/)
        emit("STD-ASCII-ART", "judgment", ln, line)     # +---+ ASCII box art
    if (line !~ /\$/ && (line ~ /(^|[[:space:]=(])(sum|prod|sqrt|frac)_/ || line ~ /[[:space:]]=[[:space:]]sum[[:space:]]/))
        emit("STD-PLAINTEXT-MATH", "judgment", ln, line)

    # Issue-only prose heuristics.
    if (tk == "issue") {
        lc = tolower(line)
        if (lc ~ /(same as[[:space:]]|as (described|mentioned|noted|shown|above)|see above|the previous issue|previous item|sibling issue|per (a[0-9]|the sibling))/)
            emit("STD-CROSSREF", "judgment", ln, line)
        if (line ~ /jit[[:space:]]+(issue|dep|gate|claim|doc|validate|query|graph|recover|init|apply|config)([[:space:]]|$)/)
            emit("STD-TRACKER-MECHANICS", "judgment", ln, line)
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
}
AWK

scan_body() {
    # kind, target, own-id, body-text
    printf '%s' "$4" | gawk -v tk="$1" -v target="$2" -v own="$3" "$AWK_BODY" >> "$raw"
}

# --- Issue pass -----------------------------------------------------------
for f in "${issue_files[@]}"; do
    sid="$(jq -r '.id[0:8]' "$f")"
    title="$(jq -r '.title // ""' "$f")"
    desc="$(jq -r '.description // ""' "$f")"

    # Title: embedded id / ordinal / conventional-commit prefix (mechanical).
    if [[ "$title" =~ ^[0-9a-fA-F]{6,}[/:] ]] \
        || [[ "$title" =~ \(jit:[0-9a-f]+\) ]] \
        || [[ "$title" =~ ^[A-Za-z]+\([^\)]*\): ]] \
        || [[ "$title" =~ ^[0-9]+[.\):][[:space:]] ]] \
        || [[ "$title" =~ [A-Za-z][0-9]+/[A-Za-z]?[0-9]*: ]]; then
        emit issue "$sid" STD-TITLE-EMBEDDED-ID mechanical 0 "$title"
    fi
    # Title: escaped angle brackets need rewording (judgment).
    if [[ "$title" == *"&lt;"* || "$title" == *"&gt;"* || "$title" == *"<"* || "$title" == *">"* ]]; then
        emit issue "$sid" STD-TITLE-ANGLE judgment 0 "$title"
    fi
    # Strategic labels must be kebab slugs, not the 8-hex short id (mechanical).
    while IFS= read -r lbl; do
        [[ -z "$lbl" ]] && continue
        ns="${lbl%%:*}"
        val="${lbl#*:}"
        case "$ns" in
            epic|story|milestone)
                if [[ "$val" =~ ^[0-9a-f]{8}$ ]]; then
                    emit issue "$sid" STD-LABEL-SLUG mechanical 0 "$lbl"
                fi
                ;;
        esac
    done < <(jq -r '(.labels // [])[]' "$f")

    scan_body issue "$sid" "$sid" "$desc"
done

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
jq -R -s '
    split("\n") | map(select(length > 0)) | map(split("\t")) |
    map({
        target_kind: .[0],
        target: .[1],
        rule: .[2],
        classification: .[3],
        line: (.[4] | tonumber),
        detail: .[5]
    }) |
    sort_by(.target_kind, .target, .rule, .line, .detail) |
    .[]
' "$raw" | jq -c '.'

total="$(wc -l < "$raw" | tr -d ' ')"
mech="$(cut -f4 "$raw" | grep -c '^mechanical$' || true)"
judg="$(cut -f4 "$raw" | grep -c '^judgment$' || true)"
echo "[scan] ${total} findings: ${mech} mechanical, ${judg} judgment" >&2
