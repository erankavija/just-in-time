# Documentation assertions leave the Rust suite — relocation evidence

Where each property the deleted Rust tests asserted now lives, and the recorded
demonstration REQ-05 requires.

All 225 relocated declarations were proven to fail their checker when violated
**before** any Rust test was deleted: each `require` / `exactly_once` phrase was
removed from its artifact and each `forbid` phrase injected into it, in a scratch
copy of the tree, and `scripts/docs-check-policy-phrases.sh` was required to exit
1 in every case.

## Where each property lives

### `crates/jit/tests/fast_docs_templates/code_review_policy_test.rs` (deleted, 14 tests)

Every entry below is a `[[document]]` or `[[value]]` declaration in
`scripts/docs-policy-phrases.toml`, checked by `scripts/docs-check-policy-phrases.sh`
as the `docs-mechanical` gate's **M9 policy phrases** step.

| Deleted test | Property | Now declared as |
| --- | --- | --- |
| `test_code_review_prompt_defines_attribution_and_fallback_policy` | 7 phrases in the review prompt | `code-review-prompt.md` / `attribution and fallback` `require` |
| `test_code_review_prompt_defines_current_evidence_and_debt_policy` | 8 phrases | `…` / `current evidence and debt` `require` |
| `test_code_review_prompt_rejects_blanket_public_api_examples` | 4 phrases in the prompt | `…` / `selective examples` `require` |
| " | 2 phrases in `AGENTS.md` | `AGENTS.md` / `selective examples` `require` |
| " | **negative**: the blanket `# Examples` requirement has not returned | `…` / `selective examples` **`forbid`** |
| `test_code_review_prompt_does_not_treat_pending_peer_review_as_a_defect` | 7 phrases | `…` / `peer-review gate evidence` `require` |
| " | **negative**: the blanket pending-gate rule has not returned | `…` / `peer-review gate evidence` **`forbid`** |
| `test_code_review_prompt_discovers_applicable_policy_in_precedence_order` | 7 phrases | `…` / `policy precedence` `require` |
| `test_code_review_prompt_bounds_and_resolves_addressable_policy` | 11 phrases | `…` / `bounded addressable policy` `require` |
| `test_code_review_prompt_checks_relationship_evidence_claims` | 9 phrases | `…` / `relationship evidence claims` `require` |
| `test_code_review_prompt_requires_complete_evidence_header_and_empty_values` | 5 header fields each occurring **exactly once** | `…` / `evidence header` **`exactly_once`** |
| " | the empty-value instruction | `…` / `evidence header` `require` |
| `test_code_review_prompt_uses_recorded_gates_and_structured_references` | 8 phrases | `…` / `recorded gates and structured references` `require` |
| " | **negative**: no repository-local invariant in a portable prompt | `…` / same topic **`forbid`** (`@/inv/pid-safety`) |
| `test_code_review_prompt_is_procedure_not_a_registry_or_engineering_rubric` | **negative**: no embedded rubric heading, no 4 copied registry statements | `…` / `procedure, not a registry copy` **`forbid`** (5) |
| `test_code_review_documentation_explains_policy_and_transport_ownership` | 6 phrases in `docs/how-to/custom-gates.md` (whitespace-normalized) | `custom-gates.md` / `review policy resolution` `require`, `normalize = true` |
| " | 3 exact findings-ownership sentences there | `custom-gates.md` / `findings ownership` `require`, `normalize = true` |
| " | **negative**: 2 phrasings that misassign parsing to the wrapper | `custom-gates.md` / `findings ownership` **`forbid`** |
| " | 6 phrases in `dev/index.md` (exact, unnormalized) | `dev/index.md` / `review policy resolution` `require` |
| " | 3 findings-ownership sentences there (normalized) | `dev/index.md` / `findings ownership` `require`, `normalize = true` |
| `test_code_review_prompt_requires_read_only_bounded_inspection` | 9 phrases | `…` / `read-only bounded inspection` `require` |
| `test_code_review_prompt_does_not_duplicate_common_findings_contract` | **negative**: 2 wrapper-contract literals absent | `…` / `shared findings contract` **`forbid`** |
| `test_code_review_gate_uses_explicit_read_only_sandbox` | the `code-review` gate's reviewer command runs `--sandbox read-only` | **`[[value]]`** `.jit/gates.toml`, `select = "gates[key=code-review].checker.env.REVIEWER_AGENT"`, `require = ["--sandbox read-only"]` |

### Package-content tests in `crates/jit/src/profile/repository_package.rs` (deleted)

All now live in `scripts/docs-check-package-content.sh`, the gate's **M8 package
content** step. It captures each package under `profiles/` with `jit profile
capture` and walks the content that manifest declares, once, for both halves.

| Deleted test | Property | Now |
| --- | --- | --- |
| `test_packaged_qualified_item_citations_resolve_from_installed_registries` | every `@/kind/id` in package content resolves from the registries that package installs | M8's citation half: the package's `keyed-array` contributions indexed per registry table, and each citation leader resolved through the live `item_kinds` configuration — a registry-first kind names the table its items live in, and its aliases lead to the same table |
| `test_jit_dogfood_package_contains_no_deferred_or_checkout_local_content` | 19 literals absent from every packaged file's source, target and text | `[[content]]` / `checkout-local and deferred content` **`forbid`** in `scripts/docs-package-content.toml` |
| `test_installed_paths_do_not_require_jq` | no packaged file names `jq` as a token | `[[content]]` / `installed paths require no jq` **`forbid_token`** |
| `test_packaged_skills_only_require_declared_project_item_kinds` | a packaged skill must not require an undeclared item kind | M8's citation half, more strictly: a `@/charter/…` address in packaged content resolves to nothing, because a markdown-first kind installs no registry — reported rather than permitted |

`docs-check-citations.sh` (M3) keeps excluding `profiles/*/assets/*`, because it
resolves against *this* repository's registries, which is the wrong domain for
content read at a destination.

Each of the 20 prohibitions was injected into package content and required to
exit 1 naming that literal, before the Rust tests were deleted.

### Sibling tests removed under REQ-01

| Deleted test | Property | Now declared as |
| --- | --- | --- |
| `doc_review_policy_test.rs::test_doc_review_prompt_defines_issue_scoped_impact_policy` | 14 phrases in `scripts/doc-review-prompt.md` | `doc-review-prompt.md` / `issue-scoped impact` `require` |
| `…::test_doc_review_gate_description_matches_issue_scoped_policy` | 10 phrases in the `doc-review` gate description | `[[value]]` `select = "gates[key=doc-review].description"` |
| `…::test_doc_review_prompt_distinguishes_leaf_footprint_from_container_footprint` | 2 phrases **inside the leaf section**, 3 **inside the container section** | two entries with `section = { after, before }` — placement, not mere presence |
| `…::test_doc_review_prompt_defines_holistic_container_checks` | 8 phrases | `doc-review-prompt.md` / `holistic container checks` `require` |
| `…::test_ai_review_wrapper_allows_passing_advisory_findings` | 1 phrase, 1 **negative** in `contrib/gates/ai-review.sh` | `ai-review.sh` / `advisory findings pass` `require` + `forbid` |
| `repository_package.rs::test_packaged_content_standard_paths_are_repository_root_relative` | 1 phrase, 1 **negative** in the breakdown analysis prompt | `.agents/skills/jit-breakdown/references/analysis-prompt.md` entry (M9) |
| `…::test_public_content_standards_redirect_links_profile_installation` | 7 phrases, 1 **negative** | `docs/reference/jit-content-standards.md` entry |
| `…::test_planning_bracket_docs_describe_builtin_review_placeholders` | `placeholder` present and 4 superseded phrases absent across 5 artifacts; 6 approval-vocabulary phrases absent across 7 artifacts | 12 entries under topics `review placeholders` and `approval vocabulary` |

## REQ-05 demonstration

Run from the worktree root against the live gate, each injection reverted with
`git checkout --` immediately after.

### Injection 1 — a required policy phrase is removed

```
$ python3 - <<'PY'
p = "contrib/gates/code-review-prompt.md"
t = open(p).read()
open(p, "w").write(t.replace("Do not attribute uncommitted changes",
                             "Never attribute uncommitted changes"))
PY
$ DOCS_FOOTPRINT=docs/ ./scripts/docs-mechanical.sh
…
== M9 policy phrases ==
MISSING: contrib/gates/code-review-prompt.md (attribution and fallback): does not state 'Do not attribute uncommitted changes'

$ echo $?
1
```

### Injection 2 — an unresolvable `@/` citation in package content

```
$ printf '\nSeeded @/rule/does-not-exist-zzz citation.\n' \
    >> profiles/jit-dogfood/assets/regions/agents-jit-guidance.md
$ DOCS_FOOTPRINT=docs/ ./scripts/docs-mechanical.sh
…
== M8 package content ==
DANGLING: jit-dogfood: AGENTS.md cites unresolved package item @/rule/does-not-exist-zzz

== M9 policy phrases ==
OK: declared policy phrases hold

$ echo $?
1
```

Both injections are also committed as re-runnable assertions in
`scripts/docs-check-selftest.sh`, which seeds them into a throwaway clone and
reads the phrase and the package region from the live declarations rather than
restating them.

## Footprint (REQ-04)

M8 and M9 take **no footprint argument** (each takes an optional manifest path,
which the self-test uses and the gate omits). Their targets are configured —
M9's by its manifest, M8's by the package directories under `profiles/` and its
own manifest — which is the
contract `.jit/gates.toml` already states for this gate: *"A checker whose
targets are fixed by configuration takes none."* `DOCS_FOOTPRINT` stays `docs/`,
so the adopter documentation surface is unchanged and no other checker's scope
moved. The gate's `inputs.roots` already cover `AGENTS.md`, `contrib`,
`profiles`, `scripts` and `dev`, so a commit touching any of them runs the gate.

## Line balance (REQ-06)

0 Rust lines added, 707 removed (`repository_package.rs` 229,
`code_review_policy_test.rs` 356, `doc_review_policy_test.rs` 120, `main.rs` 2).
