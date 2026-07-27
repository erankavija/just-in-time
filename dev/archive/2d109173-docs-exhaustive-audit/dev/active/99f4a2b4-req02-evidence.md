# 99f4a2b4 — REQ-02 execution evidence (rework round 2)

Durable, re-runnable evidence that the three `docs-mechanical` checkers satisfy
REQ-02 (each exits nonzero on a seeded defect and zero on a clean footprint),
plus the round-2 finding-specific verifications (F1–F4). Every block below is
verbatim command output captured on branch `worktree-agent-99f4a2b4`.

## Round-2 resolution table

| # | Round | Finding | Resolution (file:line at HEAD) |
|---|-------|---------|--------------------------------|
| F1 | 2 | Citation checker must catch repo-rooted dangling citations incl. extensionless, suppress non-repo/external classes, document deferrals | four-part MISSING rule `scripts/docs-check-citations.sh:91-109` (placeholder / trailing-slash / external-command / repo-root-first-segment; extension gate removed), tracked-root set derived at `scripts/docs-check-citations.sh:80`, deferral classes documented `scripts/docs-check-citations.sh:28-45` |
| F2 | 2 | Link resolver ignored reference-style Markdown links | REF_USE / REF_DEF regexes `scripts/docs-check-links.sh:75-77`, definitions + usages collected in `scan()` `scripts/docs-check-links.sh:91-135`, resolver + undefined-label finding `scripts/docs-check-links.sh:137-174`; shortcut-form deferral documented `scripts/docs-check-links.sh:19-30` |
| F3 | 2 | Self-test could destroy unrelated user work | trap removes only scratch, no `git restore` of real targets `scripts/docs-check-selftest.sh:52-53`; M5 runs inside a throwaway `git clone` `scripts/docs-check-selftest.sh:86-113` |
| F4 | 2 | No durable/recorded REQ-02 execution evidence | this file, linked via `jit doc add 99f4a2b4 dev/active/99f4a2b4-req02-evidence.md --doc-type analysis` |

## Design note (F1 deferral, per plan §2 M3)

The M3 citation check is auditor-adjudicated by design. It flags a backtick token
`MISSING:` iff it is not placeholder/glob notation, not an external/command token
(`~…`, absolute `/…`, `://`, whitespace or `|`), not a trailing-slash directory
reference, contains a `/`, its first path segment is a real tracked top-level repo
entry (live `git ls-files` set), and it does not exist on disk — regardless of
extension. Bare filenames, subdir-relative / misspelled-root citations, and
directory references are DEFERRED to the semantic doc-review reviewer; the
mechanical check cannot distinguish those from prose without false positives.
This restores the repo-root requirement whose earlier removal produced 13 false
positives on the adopter surface.

## Evidence

### 1. Binary-at-HEAD guard + install

```
$ cargo install --path crates/jit --force  (then verify binary matches HEAD)
OK: binary at HEAD (5035459a)
```

### 2. Full self-test (REQ-02: seeded-defect -> nonzero, clean -> zero)

```
$ scripts/docs-check-selftest.sh
== M2 docs-check-links.sh ==
PASS: links: seeded missing-target link is a finding (exit 1)
PASS: links: clean footprint resolves (exit 0)

== M3 docs-check-citations.sh ==
PASS: citations: dangling repo-rooted path is MISSING (exit 1)
PASS: citations: real path resolves, placeholder suppressed (exit 0)

== footprint error handling (env errors, never a false-green pass) ==
PASS: links: nonexistent footprint path is an env error (exit 2)
PASS: citations: nonexistent footprint path is an env error (exit 2)
PASS: links: unreadable footprint file is an env error (exit 2)
PASS: citations: unreadable footprint file is an env error (exit 2)
PASS: links: unreadable nested directory is an env error (exit 2)
PASS: citations: unreadable nested directory is an env error (exit 2)

== M5 docs-check-projections.sh ==
PASS: projections: fresh (rendered==staged) tree is clean (exit 0)
PASS: projections: drifted target region is a finding (exit 1)

SELFTEST: all assertions passed
(exit 0)
$ git status --porcelain   # real tree untouched by the self-test
 M scripts/docs-check-citations.sh
 M scripts/docs-check-links.sh
 M scripts/docs-check-selftest.sh
```

### 3. F1 — repo-rooted citation check

```
$ scripts/docs-check-citations.sh docs README.md INSTALL.md mcp-server/README.md web/README.md
MISSING: .jit/claims.jsonl
(exit 1)  # ONLY .jit/claims.jsonl — a real defect owned by another task

# scratch: extensionless repo-rooted path caught; external/subdir/curl deferred
# input: a `crates/jit/NOTICE` b `~/.config/x.toml` c `schemas/y.json` d `/etc/jit/config.toml`
#        curl `curl -s https://x/y | jq .z`
$ scripts/docs-check-citations.sh <scratch>
MISSING: crates/jit/NOTICE
(exit 1)
```

The 13 prior false positives (external `~/.config` and `/etc` paths, a
`curl … | jq` command, subdir-relative `schemas/…` and `lib/…` paths) are gone;
the extensionless repo-rooted `crates/jit/NOTICE` is now caught.

### 4. F2 — reference-style link resolution

```
# scratch input:
#   See [the guide][missing-label] for details.
#   Also [here][broken].
#   [broken]: ./no-such-file-zzz.md
#   Shortcut [hard] and [--json] must NOT be flagged.
$ scripts/docs-check-links.sh <scratch>
<scratch>: missing reference-definition target -> ./no-such-file-zzz.md
<scratch>: undefined reference label -> [the guide][missing-label]
(exit 1)
$ scripts/docs-check-links.sh docs README.md INSTALL.md mcp-server/README.md web/README.md   # clean surface
OK: all links and anchors resolve
(exit 0)
```

Undefined explicit reference label and broken reference-definition target are
both caught; bare shortcut prose (`[hard]`, `[--json]`) is not flagged; the real
adopter surface still resolves clean.

### 5. F3 — self-test never mutates the real repo (dummy-change survival)

```
before self-test:  sha256(AGENTS.md)=9bcc770ba2e1e0383d6c0e0d6530b64b81916d45e0f639fed2d9f5baac8c284e
 AGENTS.md | 2 ++
 1 file changed, 2 insertions(+)
self-test exit=0
after  self-test:  sha256(AGENTS.md)=9bcc770ba2e1e0383d6c0e0d6530b64b81916d45e0f639fed2d9f5baac8c284e
dummy markers still present: 2
 AGENTS.md | 2 ++
 1 file changed, 2 insertions(+)
(dummy cleaned up; AGENTS.md restored)
```

A pre-existing staged AND unstaged change to a tracked file (`AGENTS.md`)
survives a full self-test run byte-for-byte (identical sha256, both markers
present, index entry intact). The M5 projection check runs inside a throwaway
`git clone` under the scratch dir, so it never touches the real index/worktree.

### 6. shellcheck

```
$ shellcheck scripts/docs-*.sh
clean (exit 0)
```
