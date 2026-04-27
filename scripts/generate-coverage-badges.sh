#!/usr/bin/env bash
# Generate SVG coverage badges from cargo-llvm-cov JSON output.
#
# Usage: generate-coverage-badges.sh <coverage.json> <output-dir>

set -euo pipefail

COVERAGE_JSON="${1:?Usage: generate-coverage-badges.sh <coverage.json> <output-dir>}"
OUTPUT_DIR="${2:?Usage: generate-coverage-badges.sh <coverage.json> <output-dir>}"

mkdir -p "$OUTPUT_DIR"

badge_color() {
    local pct_int="${1%%.*}"
    if   (( pct_int >= 90 )); then echo "#4c1"
    elif (( pct_int >= 80 )); then echo "#a3c51c"
    elif (( pct_int >= 70 )); then echo "#dfb317"
    elif (( pct_int >= 60 )); then echo "#fe7d37"
    else                           echo "#e05d44"
    fi
}

generate_badge() {
    local label="$1" pct="$2" color="$3" outfile="$4"
    local value="${pct}%"
    local label_w=$(( ${#label} * 7 + 10 ))
    local value_w=$(( ${#value} * 7 + 10 ))
    local total_w=$(( label_w + value_w ))
    local label_x=$(( label_w / 2 ))
    local value_x=$(( label_w + value_w / 2 ))

    cat > "$outfile" <<SVGEOF
<svg xmlns="http://www.w3.org/2000/svg" width="${total_w}" height="20">
  <linearGradient id="b" x2="0" y2="100%">
    <stop offset="0" stop-color="#bbb" stop-opacity=".1"/>
    <stop offset="1" stop-opacity=".1"/>
  </linearGradient>
  <clipPath id="a"><rect width="${total_w}" height="20" rx="3" fill="#fff"/></clipPath>
  <g clip-path="url(#a)">
    <rect width="${label_w}" height="20" fill="#555"/>
    <rect x="${label_w}" width="${value_w}" height="20" fill="${color}"/>
    <rect width="${total_w}" height="20" fill="url(#b)"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11">
    <text x="${label_x}" y="15" fill="#010101" fill-opacity=".3">${label}</text>
    <text x="${label_x}" y="14">${label}</text>
    <text x="${value_x}" y="15" fill="#010101" fill-opacity=".3">${value}</text>
    <text x="${value_x}" y="14">${value}</text>
  </g>
</svg>
SVGEOF
}

COVERAGE_DATA=$(python3 -c "
import json, sys

data = json.load(open(sys.argv[1]))
crates = {
    'jit': ['/crates/jit/', 0, 0],
    'jit-server': ['/crates/server/', 0, 0],
}

for f in data['data'][0]['files']:
    filename = f['filename']
    for values in crates.values():
        if values[0] in filename:
            values[1] += f['summary']['lines']['covered']
            values[2] += f['summary']['lines']['count']

for name in sorted(crates):
    _, covered, total = crates[name]
    pct = covered / total * 100 if total > 0 else 0
    print(f'{name} {pct:.1f}')

totals = data['data'][0]['totals']['lines']
print(f'workspace {totals[\"percent\"]:.1f}')
" "$COVERAGE_JSON")

while read -r crate pct; do
    color=$(badge_color "$pct")
    generate_badge "$crate" "$pct" "$color" "${OUTPUT_DIR}/${crate}.svg"
    echo "Generated ${crate}.svg (${pct}% - ${color})"
done <<< "$COVERAGE_DATA"
