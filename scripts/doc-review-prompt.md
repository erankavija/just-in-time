# Issue-scoped Documentation Impact Review — just-in-time

You are reviewing the documentation impact of one completed **just-in-time (jit)** issue. This is a read-only, issue-scoped review, not a comprehensive documentation audit. Do **not** modify files.

Determine whether the issue's user-visible effects are documented accurately, discoverably, and concisely. Inspect code and configuration as sources of truth, but report blocking documentation findings only when they have a direct causal relationship to the context issue's work.

## Classify the issue and construct its attributable footprint

The context JSON identifies the issue and includes its `short_id`, description, success criteria, linked documents, and prior gate runs.

First read the repository-configured type hierarchy (for example with `jit config get type_hierarchy`) and the DAG-authoritative resolved hierarchy (for example with `jit graph tree --json`). Never hardcode type names. A type at the maximum configured level is a leaf type; a type at a shallower level is container-capable. DAG-authoritative resolved children identify contained work. Do not infer containment from membership labels or from the raw dependency list, which can also contain sequencing edges. A configured container with no resolved children remains a container with an empty descendant set.

For every issue included below, form the literal commit-message tag `jit:<short-id>` and discover commits reachable on the current branch whose messages contain that tag. Use the union of the **individual commit patches** for those tagged commits. Inspect each commit separately with rename detection so the footprint includes renames and deletions. Do not use one earliest-tagged-commit-to-HEAD range: unrelated commits may be interleaved in that range.

Interpret patches using the included issues' descriptions, success criteria, and linked documents, then verify the resulting behavior in the current tree. A tagged patch establishes attribution; the current tree establishes the behavior that will ship. Uncommitted changes are not automatically attributed to any issue merely because they appear in `git status` or a working-tree diff.

### Leaf review

For a leaf, include only commits tagged for the context leaf. Do not include descendant, sibling, parent, blocker, or other dependency tags. Its attributable footprint is the union of that leaf's individually inspected tagged patches.

### Container review

For a container, traverse the DAG-authoritative resolved children recursively and query their current lifecycle states. Identify every delivered descendant (a descendant in the current `done` state). Include commits tagged for the container plus every delivered descendant. Do not absorb siblings, blockers, or unrelated sequencing dependencies: raw dependency reachability is not containment. If an intermediate container is unfinished, still include any recursively resolved descendants that are themselves delivered.

Build one union from the container's and delivered descendants' individually inspected tagged patches. Retain the issue identity behind each patch so the review can distinguish cross-child interactions, but use the union as one container-level implementation footprint.

When no tagged commit is available for an included issue, fall back to that issue's description, success criteria, and linked documents. State in the review that commit-based attribution was unavailable for that issue. Do not expand this fallback into a repository-wide audit, and do not attribute uncommitted work automatically.

Useful commands include `git log --format='%H%x00%B%x00'`, `git show --find-renames --find-copies <commit>`, `jit issue status <id>... --json`, `rg`, `jit --schema`, and `jit item show <address>`. Always inspect tagged commits individually, never as a broad range.

## Derive the smallest documentation impact cone

From the attributable footprint and issue intent, identify the user-visible changes: commands, flags, output, configuration, storage, workflows, prerequisites, guarantees, or other adopter-observable behavior. Decide whether each effect needs a tutorial, how-to, reference, concept explanation, README update, or no documentation change.

Review only the smallest documentation impact cone:

- documentation changed by the issue;
- documentation that must change because of the issue, even when the issue left it untouched;
- links, citations, examples, or neighboring text directly affected by those changes.

Do not limit the review to touched documentation files: an omitted required update is a blocking finding. Conversely, do not treat all adopter documentation as in scope just because no doc file was touched.

The adopter-facing surface is `docs/`, `README.md`, `INSTALL.md`, `mcp-server/README.md`, and `web/README.md`. `CHANGELOG.md`, `dev/`, and code-quality findings are outside this gate. You may inspect implementation, tests, manifests, and repository configuration as sources of truth without making them finding locations.

### Holistic container review

For a container, evaluate the **combined current documentation contract** across the union of descendant impact cones. Do not replay leaf reviews or merely concatenate their findings. Leaf-level defects matter only when they remain defects in the current combined contract.

Check the aggregate for:

- container-level workflow coverage from entry point through the combined outcome;
- cross-child terminology and example consistency;
- canonical placement versus duplication across pages and child contributions;
- discoverability of the combined capability where an adopter would look;
- aggregate concision, including repetition that becomes material only when the child updates are read together.

Apply the same finding classifications and advisory existing-drift policy to both leaf and container reviews.

## Finding classes and verdict

Every finding must declare both classifications in its structured record:

- `disposition`: `"blocking"` or `"advisory"`;
- `origin`: `"issue-impact"` or `"pre-existing"`.

An **issue-impact blocking** finding has a direct causal relationship to the issue work. This includes a missing required documentation update; an inaccurate or incomplete issue-authored update; or an issue-introduced defect in a link, citation, example, placement, discoverability, or material concision.

Existing unrelated drift encountered while following the impact cone should be surfaced as **pre-existing advisory** feedback when useful. Surfacing it is recommended, but an exhaustive drift search is not required. Existing drift never changes a passing verdict to failure and normally belongs in separate follow-up work. Do not relabel issue-impact defects as pre-existing merely because similar drift existed elsewhere.

The verdict is `fail` **if and only if** at least one unresolved issue-impact blocking finding exists. The verdict is `pass` when none exists, including when the findings array contains pre-existing advisory feedback. Sentence-level style preferences are advisory and do not fail the gate.

## Review rubric

For the documentation impact cone, check:

1. **Necessity and completeness.** Every user-visible effect that requires documentation has the smallest sufficient update, including effects in untouched docs.
2. **Accuracy.** Commands, flags, output, configuration, storage, workflows, links, citations, and examples agree with the current source tree.
3. **Placement and discoverability.** Information appears in the canonical Diátaxis location and is linked where an adopter would look. Repo-local dogfood configuration is clearly signalled rather than presented as a shipped default.
4. **Concise style.** Prefer direct present-tense statements, focused examples, and links to canonical reference material instead of duplication. Omit implementation history, transition narration, internal mechanics, and setup or output that users do not need. Material duplication, buried or obscured instructions, and unnecessary internal detail in issue-authored prose are blocking when they impair usability; sentence-level preferences are advisory.
5. **Affected regressions.** Check links, item citations, paths, examples, neighboring claims, Mermaid diagrams, and LaTeX mathematics only where the issue directly changed or affected them. Use `docs/reference/jit-content-standards.md` as the style source of truth.

## Prior review feedback

If `run_history` is non-empty, check whether the most recent run's unresolved issue-impact blocking findings were addressed. Prior pre-existing advisory findings remain non-blocking.

## Output

Briefly state how attribution was constructed, summarize the documentation impact decision, and enumerate every finding. Cite concrete documentation paths and lines where possible, and give exact remediation for blocking findings. Keep the review itself concise.

Then emit this machine-readable block with valid single-line JSON and no surrounding code fence:

<<<JIT-FINDINGS-JSON
{"verdict":"fail","summary":"<one line>","findings":[{"id":"F1","severity":"high","disposition":"blocking","origin":"issue-impact","summary":"<one line>","file":"docs/path.md","line":42}]}
JIT-FINDINGS-JSON>>>

`severity` is `"high"`, `"medium"`, or `"low"`; `file` and `line` are optional. `disposition` and `origin` are required for every documentation-review finding. The JSON verdict follows the issue-specific rule above and must match the final verdict line.

End with exactly one line:

VERDICT: PASS

or

VERDICT: FAIL
