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
# Link forms covered:
#   * inline            [text](target)
#   * reference — full  [text][label]   + collapsed  [text][]  usages, resolved
#     against link-reference definitions  [label]: target  in the same document;
#     an undefined label used in explicit `][` reference syntax is a finding, and
#     a definition whose target/anchor does not resolve is a finding.
# Bare shortcut usages `[label]` (single bracket, no `][` and no following `(`)
# are deliberately NOT treated as broken links when undefined: per CommonMark an
# undefined shortcut reference is literal text, so flagging it would false-
# positive on ordinary bracketed prose (`[hard]`, `[--json]`, `[0-9]`, TOML
# section headers like `[type_hierarchy]`). A shortcut that DOES match a
# definition is a real link; its target is already checked via that definition.
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

# A footprint entry that resolves to nothing — nonexistent OR unreadable — is a
# usage/environment error, not a clean pass. Silently skipping a mistyped or
# unreadable path would let the gate go false-green without checking anything.
for r in roots:
    if not os.path.isfile(r) and not os.path.isdir(r):
        print(f"docs-check-links: footprint path does not exist: {r}", file=sys.stderr)
        sys.exit(2)
    # Files need read access; directories additionally need traversal (execute).
    need = os.R_OK | (os.X_OK if os.path.isdir(r) else 0)
    if not os.access(r, need):
        print(f"docs-check-links: footprint path is not readable: {r}", file=sys.stderr)
        sys.exit(2)

# Collect markdown files from the footprint: files taken as-is, directories
# walked for *.md. os.walk swallows traversal errors (e.g. an unreadable nested
# directory) by default, which would silently shrink the footprint and let the
# gate go false-green; the onerror handler turns any such error into exit 2.
def _walk_error(err):
    print(f"docs-check-links: cannot traverse footprint: {err}", file=sys.stderr)
    sys.exit(2)

files = []
for r in roots:
    if os.path.isfile(r):
        files.append(r)
    elif os.path.isdir(r):
        for dp, _, fns in os.walk(r, onerror=_walk_error):
            files += [os.path.join(dp, f) for f in fns if f.endswith('.md')]

INLINE_CODE = re.compile(r'`[^`]*`')
HEADING = re.compile(r'#{1,6}\s+(.*)')
LINK = re.compile(r'\[[^\]]*\]\(([^)]+)\)')
# Full / collapsed reference usage: [text][label] (label may be empty=collapsed).
REF_USE = re.compile(r'\[([^\]]*)\]\[([^\]]*)\]')
# Link-reference definition: up to 3 leading spaces, [label]: target [optional title]
REF_DEF = re.compile(r'^ {0,3}\[([^\]]+)\]:\s+(\S+)')


def github_slug(text):
    s = re.sub(r'[^\w\s-]', '', text.strip().lower())
    return s.replace(' ', '-').strip('-')


def norm_label(text):
    # Markdown reference labels match case-insensitively with internal
    # whitespace collapsed.
    return ' '.join(text.split()).lower()


def scan(path):
    # Parse one markdown file, skipping fenced code blocks — a heading, link,
    # reference usage, or definition inside ``` is not rendered by GitHub, so it
    # neither defines an anchor/label nor is a real link. Returns:
    #   slugs     heading anchors defined in this file
    #   links     inline-link targets
    #   deftgts   link-reference definition targets (resolved like inline links)
    #   deflbls   set of defined reference labels (normalized)
    #   refuses   (text, label) of explicit `][` reference usages
    slugs, links, deftgts, deflbls, refuses, fence = set(), [], [], set(), [], False
    try:
        text = open(path, encoding='utf-8').read()
    except OSError as e:
        # A file we collected from the footprint but cannot read is an
        # environment error, not an empty (clean) parse — surface it as exit 2
        # rather than letting the gate go false-green.
        print(f"docs-check-links: cannot read {path}: {e}", file=sys.stderr)
        sys.exit(2)
    for line in text.splitlines():
        if line.lstrip().startswith('```'):
            fence = not fence
            continue
        if fence:
            continue
        h = HEADING.match(line)
        if h:
            slugs.add(github_slug(h.group(1)))
        # A link-reference definition is matched on the raw line (it is a
        # line-level construct); strip an angle-bracket wrapper from the target.
        d = REF_DEF.match(line)
        if d:
            deflbls.add(norm_label(d.group(1)))
            deftgts.append(d.group(2).strip().lstrip('<').rstrip('>'))
            continue
        # Inline-code spans (`...`) are not links; strip them first so a
        # bracketed example inside backticks is not treated as a live link.
        clean = INLINE_CODE.sub('', line)
        for tgt in LINK.findall(clean):
            links.append(tgt.strip())
        for text, label in REF_USE.findall(clean):
            # Collapsed form [text][] uses the text as its label.
            refuses.append((text, label if label.strip() else text))
    return slugs, links, deftgts, deflbls, refuses


parsed = {f: scan(f) for f in files}

bad = []


def resolve_target(f, d, t, kind):
    # Resolve one repo-relative link/definition target (path + optional #anchor)
    # against the tree, appending to `bad` on failure. `kind` labels the finding.
    if t.startswith(('http://', 'https://', 'mailto:')):
        return
    path, _, anchor = t.partition('#')
    if path == '':
        tgt = f
    else:
        tgt = os.path.normpath(os.path.join(d, path))
        if not os.path.exists(tgt):
            bad.append(f'{f}: missing {kind} target -> {t}')
            return
    if anchor:
        if tgt in parsed:
            tgt_slugs = parsed[tgt][0]
        elif tgt.endswith('.md'):
            tgt_slugs = scan(tgt)[0]
        else:
            # Anchor into a non-markdown target (e.g. a `#L10` line anchor into
            # source): not a heading slug, cannot resolve.
            return
        if anchor not in tgt_slugs:
            bad.append(f'{f}: unresolved {kind} anchor -> {t}')


for f in files:
    d = os.path.dirname(f)
    slugs, links, deftgts, deflbls, refuses = parsed[f]
    for t in links:
        resolve_target(f, d, t, 'link')
    for t in deftgts:
        resolve_target(f, d, t, 'reference-definition')
    # An explicit `][` reference usage whose label has no matching definition in
    # the same document is a broken reference link.
    for text, label in refuses:
        if norm_label(label) not in deflbls:
            bad.append(f'{f}: undefined reference label -> [{text}][{label}]')

if bad:
    print('\n'.join(bad))
    sys.exit(1)
print('OK: all links and anchors resolve')
PY
