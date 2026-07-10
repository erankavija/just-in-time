#!/usr/bin/env bash
set -euo pipefail

# docs-check-links — markdown link + heading-anchor resolver.
# Mechanical check M2 of the `docs-mechanical` gate (epic 2d109173).
#
# Resolves every intra-repo markdown link and every intra-document heading
# anchor in a footprint against the live tree. Both comparison sides are
# derived at runtime — links and headings are read from the same files, and
# targets are tested against the working tree — so the check encodes no
# product facts (no path allowlist, no heading inventory).
#
# GitHub heading-anchor reproduction: lowercase, drop every character that is
# not a word char / whitespace / hyphen, then map spaces to hyphens one-to-one
# (so an em-dash gap "  " collapses to "--"), and trim leading/trailing
# hyphens. LIMITATION: this is a first-pass resolver. It does NOT synthesize
# GitHub's numeric "-1"/"-2" disambiguation suffixes for repeated identical
# headings, so an "unresolved anchor" reported against a duplicated heading is
# human-adjudicated, not an automatic defect. It also does not resolve line
# anchors (e.g. `#L10`) into non-markdown source targets — those are skipped.
#
# Usage:
#   docs-check-links.sh PATH [PATH ...]
# The footprint is a REQUIRED space-separated list of files/dirs — the checker
# encodes no default path list (that would be a product fact, REQ-01). The gate
# entrypoint (docs-mechanical.sh) derives the whole-surface footprint live and
# passes it in; area audits pass their own.
#
# Exit codes:
#   0 — every link and anchor resolves  (prints "OK: all links and anchors resolve")
#   1 — one or more unresolved links/anchors (each printed as a finding)
#   2 — usage/environment error

if [ "$#" -eq 0 ]; then
  echo "usage: docs-check-links.sh PATH [PATH ...]" >&2
  exit 2
fi

exec python3 - "$@" <<'PY'
import os, re, sys

roots = sys.argv[1:]
if not roots:
    print("usage: docs-check-links.sh [PATH ...]", file=sys.stderr)
    sys.exit(2)

# Collect markdown files from the footprint: files taken as-is, directories
# walked for *.md. A non-existent footprint entry is skipped here — path
# existence is the citation checker's (M3) concern, not this resolver's.
files = []
for r in roots:
    if os.path.isfile(r):
        files.append(r)
    elif os.path.isdir(r):
        for dp, _, fns in os.walk(r):
            files += [os.path.join(dp, f) for f in fns if f.endswith('.md')]

INLINE_CODE = re.compile(r'`[^`]*`')
HEADING = re.compile(r'#{1,6}\s+(.*)')
LINK = re.compile(r'\[[^\]]*\]\(([^)]+)\)')


def github_slug(text):
    s = re.sub(r'[^\w\s-]', '', text.strip().lower())
    return s.replace(' ', '-').strip('-')


def scan(path):
    # Parse one markdown file into (heading slugs, link targets), skipping
    # fenced code blocks for both — a heading or link inside ``` is not
    # rendered by GitHub, so it neither defines an anchor nor is a real link.
    slugs, links, fence = set(), [], False
    try:
        text = open(path, encoding='utf-8').read()
    except OSError:
        return slugs, links
    for line in text.splitlines():
        if line.lstrip().startswith('```'):
            fence = not fence
            continue
        if fence:
            continue
        h = HEADING.match(line)
        if h:
            slugs.add(github_slug(h.group(1)))
        # Inline-code spans (`...`) are not links; strip them first so a
        # bracketed example inside backticks is not treated as a live link.
        for tgt in LINK.findall(INLINE_CODE.sub('', line)):
            links.append(tgt.strip())
    return slugs, links


parsed = {f: scan(f) for f in files}

bad = []
for f in files:
    d = os.path.dirname(f)
    for t in parsed[f][1]:
        if t.startswith(('http://', 'https://', 'mailto:')):
            continue
        path, _, anchor = t.partition('#')
        if path == '':
            tgt = f
        else:
            tgt = os.path.normpath(os.path.join(d, path))
            if not os.path.exists(tgt):
                bad.append(f'{f}: missing target -> {t}')
                continue
        if anchor:
            if tgt in parsed:
                tgt_slugs = parsed[tgt][0]
            elif tgt.endswith('.md'):
                tgt_slugs = scan(tgt)[0]
            else:
                # Anchor into a non-markdown target (e.g. a `#L10` line
                # anchor into source): not a heading slug, cannot resolve.
                continue
            if anchor not in tgt_slugs:
                bad.append(f'{f}: unresolved anchor -> {t}')

if bad:
    print('\n'.join(bad))
    sys.exit(1)
print('OK: all links and anchors resolve')
PY
