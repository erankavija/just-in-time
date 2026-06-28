# Addressing v2 + Rule/Gate as Addressable Items (pre-plan design brief)

**Status:** Design brief, not a plan. Captures the design discussion that followed
epic `90a2dbfd` (declarative kinds over sources). Input for a future `jit-plan`
run; it does NOT define success criteria or create issues. Open points are listed
explicitly in section 8 for the formal plan to resolve.

**Origin:** REQ-07 of epic `90a2dbfd` (rule/gate-as-item) was marked `[aspirational]`
and deferred (decision D7). Rule/gate addressability was a primary motivator for that
epic, so the owner reopened it. The interview below grew it from "make rule/gate
addressable" into an addressing-v2 redesign.

## 1. Why REQ-07 was deferred, and what is now unblocked

- **Original blocker (D7):** rule self-ids contain colons (`default:label-format`,
  `default:namespace-unique:resolution`), which collide with the label
  `namespace:value` separator, so `enforces:@/<rule>` could not be authored.
- **Now unblocked by 90a2dbfd:** REQ-02 added a config-declared toml *source
  descriptor* and REQ-04 made `jit init` author the `[item_kinds]` table. Declaring
  a new registry-first kind is now pure config. The only remaining work is the
  addressing/grammar layer plus the rule/gate substrate specifics.

## 2. Findings from investigation (grounded)

- **`@` is not valid in label values today.** `crates/jit/src/labels.rs:24`:
  `^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._/-]*$`. The value class has no `@`. So
  project-scoped links (`enforces:@/INV-DAG-ACYCLIC`) are NOT authorable as labels
  at all right now; only issue-scoped links work (`per:<issue>/D-01`, no `@`). This,
  not the colon, is the real gap. Any rule/gate linking work forces a value-grammar
  change to admit `@`.
- **`@` is already a parsed scope sentinel** elsewhere: `PROJECT_SCOPE_SENTINEL = "@"`
  and `Scope::parse("@") => Scope::Project` (`crates/jit/src/domain/item.rs:62`,
  `:114`). The label grammar simply never caught up to it.
- **Label parsing splits on the first colon only:** `parse_label` uses
  `splitn(2, ':')` (`crates/jit/src/labels.rs:77`). So a value may contain further
  separators safely; `/`-delimited paths in values are already parse-safe (the value
  class already allows `/`).
- **Rule names are colon-namespaced with real structure:** origin then family then
  optional param. Seed set (`.jit/rules.toml`): `default:label-format`,
  `default:namespace-registry`, `default:type-hierarchy-known`,
  `default:namespace-unique:{resolution,type,team}`, `default:orphan-leaf`,
  `default:strategic-consistency`, and `bracket:coverage-preview` (origin `bracket`,
  not `default`). The `default:` prefix is also baked into code
  (`crates/jit/src/validation/defaults.rs` asserts `name.starts_with("default:")`
  and builds `format!("default:namespace-unique:{name}")`,
  `<default:{name}>` reference/path strings).
- **A concrete cross-kind collision exists:** rule `bracket:coverage-preview` and
  gate key `coverage-preview` would both want `@/coverage-preview` in a flat
  per-scope id space. Project self-ids must be unique per scope (`DuplicateSelfId`),
  so a flat scheme that strips prefixes breaks. The origin prefix currently does
  real disambiguation work.
- **Gate keys are already clean** (no colons): `cargo-ci`, `code-review`, `fmt`,
  `clippy`, `tests`, `jit-validate`, `coverage-preview`, `breakdown-review`,
  `plan-review`, `tdd-reminder`, `npm-ci`, `cargo-ci-features`, `repo-validate`. Only
  rules carry colons.
- **Gates are JSON, not TOML:** `.jit/gates.json`. The REQ-02 source descriptor reads
  TOML only, so a `gate` kind needs either a JSON descriptor or a gates JSON->TOML
  migration.
- **`enforced-by` bindings today** (`.jit/invariants.toml`): two rules
  (`default:label-format`, `default:namespace-registry`) and one gate (`cargo-ci`,
  twice). Small surface to rebind.

## 3. Goals (owner-selected)

Primary uses for rule/gate items:
1. Author `enforces:@/<rule-or-gate>` on work, resolved like `satisfies:`/`per:`/`mitigates:` today.
2. Uniform `enforced-by` resolution: every invariant binding resolves to a real addressable item.
3. Render a rules/gates reference doc from the registry (like the CLAUDE.md invariants projection, REQ-06/REQ-08).

Explicitly NOT a goal this round: re-introducing enforcement-drift accounting
(REQ-05 removed that direction; leave it removed unless a later, separate decision
revives it).

## 4. Converged design decisions (this discussion)

- **One uniform, recursive address rooted at the project**, colon-free, `/`-path form:
  - project items: `@[<project>]/<kind>/<self-id>`
    (e.g. `@/rule/label-format`, `@just-in-time/gate/cargo-ci`, `@/invariant/DAG-ACYCLIC`)
  - issue items: `@/issue/<short-id>/<kind>/<self-id>`
    with `<short-id>/<self-id>` retained as sugar.
- **The colon is reserved solely for the label `namespace:value` separator.** No
  colon overloading anywhere in values. (Rejected: widening the value grammar to
  allow `:` in values; parse-safe but reads ambiguously and keeps legacy colon ids.)
- **`/`-paths carry kind and scope.** The grammar already allows `/`; the change is
  to admit `@` (and `@<project>`) at the start of a value and to define the segment
  structure.
- **`@<project>` is a named-project generalization** of the existing `@` sentinel.
  Bare `@` is the local/relative shorthand; `@<project>` names a specific project.
  Symmetric with the issue scope naming a specific issue.
- **Multi-jit: address form now, resolution later.** Parser and grammar accept
  `@<project>`; only the local project resolves; non-local refs are parse-valid but
  return a clear "not resolvable (no federation)" error. No project registry /
  federation built this round.
- **`issue` is a reserved first-class segment, not an `[item_kinds]` entry.** Issues
  are tracked entities (state/gates/deps); item kinds are projected referents from a
  source. The address namespace is uniform, but `issue/<id>` is built-in while the
  `[item_kinds]` table declares projected kinds.
- **rule + gate become addressable kinds.** `rule` over `rules.toml` (fits the REQ-02
  toml descriptor); `gate` over a migrated `gates.toml` (gates move JSON->TOML, "no
  legacy left").
- **`enforced-by` rebinds** to the new addresses (`@/rule/...`, `@/gate/...`).
- **Re-address all kinds uniformly**, including existing invariant/requirement/
  decision/risk; migrate this repo's existing links. (Rejected: applying the scheme
  only to new kinds; the owner wants uniformity, no legacy.)
- **Self-id ergonomics matter:** prefer clean ids; the kind segment removes the need
  for colon-prefixed rule names, so rule ids can become readable slugs.

## 5. Target grammar (sketch, for the plan to finalize)

```
qualified-id := project-item | issue-item
project-item := scope "/" kind "/" self-id
issue-item   := scope "/" "issue" "/" short-id "/" kind "/" self-id
              | short-id "/" self-id            ; sugar, kind inferred by id-pattern
scope        := "@" [ project-name ]            ; "@" = local project
project-name := [a-z][a-z0-9-]*                 ; e.g. just-in-time
kind         := [a-z][a-z0-9-]*                 ; invariant | rule | gate | definition | requirement | decision | risk
self-id      := kind-defined (id-pattern)
```

Label value grammar must change from `[a-zA-Z0-9][a-zA-Z0-9._/-]*` to also admit a
leading `@`, the project-name, and the deeper `/`-path. The colon split
(`splitn(2, ':')`) is unchanged: exactly one colon, the label namespace separator.

Example labels under the scheme:
- `enforces:@/rule/label-format`
- `enforces:@just-in-time/gate/cargo-ci`
- `per:@/issue/a1b2c3/decision/D-01` (canonical) or `per:a1b2c3/D-01` (sugar)

## 6. Suggested decomposition (rough; jit-plan refines)

1. Addressing core: scope + kind-segmented qualified-id mint/parse/resolve; admit `@`
   and `@<project>` (resolve local only); sugar expansion + id-pattern inference.
2. Label value-grammar change + re-split-site audit.
3. `rule` kind over `rules.toml`; rule-identity cleanup (origin out of the id;
   resolve the coverage-preview vs gate clash).
4. `gate` kind: migrate `gates.json` -> `gates.toml`; declare the kind.
5. `enforced-by` rebind in `invariants.toml` to the new addresses.
6. Uniform re-addressing of existing kinds + migration of this repo's labels/docs/tests.
7. Rules/gates reference projection (reuse the REQ-08 style system).
8. Multi-jit address form (parse/validate `@<project>`, resolve-error for non-local).

## 7. Non-goals (for the eventual epic)

- Cross-jit federation / actual remote resolution (only the address form lands now).
- Re-introducing enforcement-drift accounting.
- Rewriting the code/regex/const that physically holds a definition's value (carried
  from epic `90a2dbfd` out-of-scope: jit makes things addressable, it does not inject
  values).

## 8. Open points to resolve in the formal jit-plan

1. **Value-grammar change, exact form.** New regex admitting `@`, `@<project>`, and
   the deeper `/`-path; enumerate and audit every site that re-splits a value (on
   `:`, `/`, or `@`) so the wider grammar does not break parsing or resolution.
2. **Rule identity refactor.** Move origin (`default`/`bracket`) out of the self-id
   into a field; update the merge/override logic and the `default:` assumptions in
   `defaults.rs` (incl. `format!`/reference/path strings). Decide the slug scheme for
   the `namespace-unique:{param}` family (e.g. `namespace-unique-resolution`). Resolve
   the `coverage-preview` rule vs gate-key clash (rename the rule, or rely on the kind
   segment making `@/rule/coverage-preview` distinct from `@/gate/coverage-preview`).
   Confirm whether the kind segment alone removes the need to rename, or a rename is
   still wanted for clarity.
3. **Sugar + inference rules.** Define `<short-id>/<self-id>` expansion to
   `@/issue/<short-id>/<kind>/<self-id>`. Specify kind inference by id-pattern,
   ambiguity handling (overlapping patterns), and whether sugar is also allowed for
   project items (`@/<self-id>` without kind).
4. **`issue` segment status.** Confirm `issue` is reserved (not an item kind). Decide
   whether `@/issue/<id>` resolves to an addressable view of the issue, and to which
   fields, or whether it is only an address prefix for issue-scoped items.
5. **Project identity.** Where the canonical project name lives (index.json?),
   uniqueness, rename handling, validation. UX for non-local `@<project>` refs
   (parse-valid, resolve-error wording). Whether `@` must equal the declared name.
6. **Gate substrate migration.** `gates.json` -> `gates.toml` schema mapping (key,
   title, description, stage, mode, checker, priority, prompt_file, env). The gate
   item descriptor (typed fields like mode/stage). Confirm no consumer still reads
   `gates.json` (single-consumer, no backward-compat per prior decisions).
7. **`enforced-by` migration mechanics.** Rebind the four current bindings; define the
   descriptor link-field mapping if `enforced-by` should also surface as an
   `enforces:` item link, vs staying a typed binding consumed only by validation.
8. **Reference projection.** Target file(s) and render style for the rules/gates
   reference; reuse the REQ-08 `style` field or add a rules-specific renderer.
9. **Re-addressing blast radius + migration.** Scan every existing qualified-id link
   (`enforces:`/`per:`/`satisfies:`/`mitigates:`/`resolves:`) across this repo's
   labels, docs, tests, and `.jit/` data; decide a migration approach (script vs
   manual) and how to keep `jit validate` green through the transition.
10. **`satisfies:` / label-coverage interaction.** `satisfies:REQ-01` is a
    coverage-credit label (matched against a container criterion id), not a
    qualified-id link. Confirm it is unaffected by the address change, or define how
    coverage ids map under the uniform scheme.
11. **Backward-compat window.** Whether old `@/INV-*` and `<issue>/REQ-*` addresses
    must keep resolving during migration, or it is a clean cut (owner leans clean /
    no-legacy; confirm).
12. **Scope of one effort.** Is this one epic, or a milestone spanning several epics
    (addressing core vs rule/gate vs re-address migration vs multi-jit)? Sequencing
    and gating.

## 9. References

- Epic `90a2dbfd` design doc: `dev/active/90a2dbfd-kinds-over-sources.md`
  (project-scoped item taxonomy, two substrates, REQ-07 rationale).
- Epic `90a2dbfd` completion report: `dev/archive/features/90a2dbfd-completion-report.md`.
- Key code: `crates/jit/src/labels.rs` (grammar/parse), `crates/jit/src/domain/item.rs`
  (`Scope`, `PROJECT_SCOPE_SENTINEL`, kind/descriptor model, `resolve_item_kinds`),
  `crates/jit/src/commands/item.rs` (`project_items`, link resolution),
  `crates/jit/src/validation/defaults.rs` (`default:` rule convention),
  `crates/jit/src/validation/projection.rs` (registry render),
  `.jit/rules.toml`, `.jit/gates.json`, `.jit/invariants.toml`.
