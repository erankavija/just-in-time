# Plan: Give every identity and closed vocabulary one semantic type (2195c98f)

> Planning node: 8aa53c43. Authoritative graph:
> [breakdown.json](breakdown.json).

## Outcome and criterion approach

Every criterion types a value at its parse boundary and deletes the restatements it replaces (`@/inv/semantic-types`, `@/charter/D-18`). Each entry lands green. Most families need no bridge. The `Issue.id` flip and the event envelope do need one, so each is staged behind a named temporary alias that a later entry in this epic removes.

| Criterion | Approach | Evidence / open gap |
|---|---|---|
| REQ-01 | `issue-identity-newtype` is adopted behind `alias-staging-bridge`. The CLI and server entry points resolve first, then the command families, then repository-state, validation and output, then storage and the domain. Test suites are staged separately, and the flip comes last. Gate-run ids and gate keys follow once the flip has landed. | Investigation, identity §4 and §7 |
| REQ-02 | `issue-id-prefix` types the shared resolver that 16d63038 creates. One entry owns the CLI parse and one the server parse (DEC-24); executor methods then take `IssueId`. | Identity C7, REQ-02 |
| REQ-03 | Each staged family replaces its truncation sites with `short_id()`. The server serves `short_id`, and the web renders it. | Identity C8; DEC-12 |
| REQ-04 | `Option<&IssueId>` over `EventBody`, once the envelope has landed. | Identity C9 |
| REQ-05 | `event-body-view` migrates matchers, identity reads and constructions first: production code, then unit tests and integration tests. The `event-envelope` swap follows. | Identity §4 envelope probe |
| REQ-06 | The agent becomes an `Assignee`, then come `LeaseId` and `WorktreeId`, then the lease's `IssueId`, which also deletes the short-id tolerance. | Identity C3; DEC-11, DEC-15 |
| REQ-07 | `virtual-path-constructors` adds issue-record, gate-run and schema constructors. Well-known-file literals are swept in four module families. | Identity C10; DEC-14 |
| REQ-08 | One predicate on classified `VirtualPath` targets. | Profile C5, REQ-08 |
| REQ-09 | `ProfileId` moves to `domain::identity`, and dependency resolution compares values. | Profile REQ-09; DEC-06 |
| REQ-10 | `wire-vocabulary-macro` plus one entry per vocabulary, including the DEC-21 twins. | Vocab C1–C10 |
| REQ-11 | `system-actor-constants`, then the gate-run actor, the agent identity and the query filter. | Identity C12–C13; DEC-19 |
| REQ-12 | State and priority value lists come from the macro. Projections are typed. | Vocab C7, C10; DEC-20 |
| REQ-13 | Event tags and artifact-plan codes go through the macro. `PlanError` derives `thiserror`. | Vocab C13–C15 |
| REQ-14 | `Ord` on `Priority`, ordered after aa423d02. | Vocab C17 |
| REQ-15 | `compiled-graph-assertions`: one entry per assertion kind, with `id-pattern` shared. | Vocab C18–C20 |
| REQ-16 | One constructor first, with the existing policies kept; then the fallback goes and the callers propagate its error. | Vocab C21; DEC-22 |
| REQ-17 | Each staging entry's REQ-01 owns the raw identity fields of its files, and the staged footprints cover each file that holds an issue id (CLI-process tests, which exchange ids with the binary as strings, are boundary). The flip's compile errors catch misused values. The `Debug`-render sweep closes the vocabulary remainder. No grep enforcement check is added. | a122b9b3 precedent |

## Shared architectural contracts

### `identity-newtype-convention` [plan-fixed] — Identity newtype shape

Identity types live in `crates/jit/src/domain/identity.rs` and are imported as `crate::domain::identity::<Type>`, following the 842b0ac6 module-path convention. Each type has a private inner value and a validating `TryFrom<&str>`/`FromStr` with a `thiserror` error that names the input. Serde goes through `#[serde(try_from = "String", into = "String")]`, and `#[schemars(with = "String")]` keeps the published schema a plain string. `as_str()` is the only borrowed string view, and `Display` prints the same text. UUID-backed ids (`IssueId`, `GateRunId`, `LeaseId`) accept only the canonical lowercase hyphenated form, checked as `to_string() == input`. Token ids (`GateKey`, `ProfileId`) use the shared lowercase-kebab predicate, and `WorktreeId` accepts a non-empty token without whitespace.

### `issue-identity-newtype` [implementation-produced] — Validated issue id

A canonical-string newtype (DEC-07). Its trait set is exactly `Display` (the same text as `as_str()`), `Debug`, `Clone`, `PartialEq`/`Eq`, `PartialOrd`/`Ord`, `Hash`, `Serialize`/`Deserialize` and `JsonSchema`, and its methods are `as_str()` and `short_id() -> &str`. It has no `Deref`, `Borrow<str>` or string comparison. It sits beside the bridge under a staging name until the flip gives it the name `IssueId`.

### `alias-staging-bridge` [implementation-produced] — Temporary IssueId alias

`pub type IssueId = String;` at the final path, plus an `IssueIdStaging` trait that gives `String` a `short_id()`. Each staging entry, test suites included, uses issue ids only through the newtype's trait set and methods, so its code compiles unchanged at the flip. Parameters are retyped outside-in, so each caller is already typed when its callee changes: test suites first, then the shared harness, then the CLI and server entry points and command families, then inner layers. A caller holding `&IssueId` still reaches a callee taking `&str` through deref coercion. This is a named bridge with a tracked removal condition: the flip entry deletes it, and that entry depends transitively on every staging entry (`@/inv/canonical-cutover`, `@/inv/convention-convergence`).

### `issue-id-prefix` [implementation-produced] — Parsed id prefix

`IssueIdPrefix` owns `MIN_ID_PREFIX_LENGTH` and the normalisation rules of the shared resolver. Resolution maps `&IssueIdPrefix` to `IssueId`. Parsing produces exactly two error classes: a prefix that normalises below the minimum length gets the resolver's invalid-prefix error before any record is read, and well-formed-length input outside the alphabet or matching nothing keeps not-found, exit 3 (DEC-23). The landed 16d63038 contract governs where they differ. Only the CLI and server entry points parse and resolve; executor and internal methods take `IssueId` (DEC-24). Interim call-site parses exist only as `prefix-call-site-bridge`. `IssueIdPrefix` implements an infallible `From<&IssueId>`, since a full id is a valid prefix. Two cases resolve inside: dependency removal takes an `IssueIdPrefix` and matches it against the issue's own dependency list, so a dangling dependency stays removable, and callers holding an `IssueId` convert it, and item-address issue segments decoded from documents, a file-decoding boundary, hold `IssueIdPrefix` and resolve where the address is used.

### `prefix-call-site-bridge` [implementation-produced] — Interim prefix parses

To land the resolver retyping green before the CLI and server boundary exists, `prefix-resolution` adds an `IssueIdPrefix` parse at each existing production `resolve_issue_id` call. These call-site parses are a named temporary bridge (`@/inv/canonical-cutover`). The removers are the two boundary entries, the five command-family entries, `scoped-issue-id-arguments` and `lease-issue-identity`, each with a grep-checkable criterion that its files hold no production `IssueIdPrefix` parse and no production `resolve_issue_id` call. Every interim site lies in a remover's footprint. The removal condition is checked by the flip: no production `IssueIdPrefix` parse remains outside the CLI and server entry points and the item-address decoder, and the only production `resolve_issue_id` call under `commands/` is the item-address resolution in `commands/item.rs`.

### `test-issue-id-fixture` [implementation-produced] — Canonical test ids

One deterministic fixture in `test_utils` that maps a seed to a distinct canonical `IssueId`. Unit tests and integration tests both use it (`@/inv/shared-test-contracts`).

### `event-envelope` [plan-fixed] — Event envelope shape

`struct Event { id, timestamp, #[serde(flatten)] body: EventBody }`, where `EventBody` is `#[serde(tag = "type", rename_all = "snake_case")]`. Drafts build `EventBody`. In production only the finalizer, event decoding and `EventTag::sample` (used by the generated reference and the events command) build `Event`. The probe in the investigation showed that canonical log bytes are unchanged and `parse_known_events` decodes the envelope unmodified. Flatten forbids `deny_unknown_fields` on `Event`.

### `event-body-view` [implementation-produced] — Temporary body view

`pub type EventBody = Event;` plus `Event::body()`, `into_body()` and `timestamp()` beside `id()`, and a `test_utils` helper that stamps an id and a timestamp onto a body. Matchers move to `match event.body() { EventBody::X {..} }`, no pattern binds a variant's `id` or `timestamp`, and constructions outside `domain/types.rs` go through draft constructors or the helper, while the enum is unchanged. The envelope entry replaces the alias with the real body enum and deletes the alias.

### `wire-vocabulary-macro` [implementation-produced] — Closed wire vocabulary declaration

A crate-local `macro_rules!` in `domain/vocabulary.rs` takes `Variant = "wire"` entries. It emits the enum with per-variant serde renames, `const fn as_str`, `ALL`, an exact `FromStr` and `Display`. One owning test proves the macro (`@/inv/orthogonal-tests`). A consumer keeps sorting by `as_str()` wherever the declaration order and the alphabetical order differ.

### `exact-token-parsing` [plan-fixed] — Exact canonical tokens

A closed vocabulary parses only the token its serde form emits: no case folding, no alias, and no `md` (DEC-17). Every token list derives from `ALL`.

### `virtual-path-constructors` [plan-fixed] — Typed path constructors

`VirtualPath::issue_record(&IssueId)` and `VirtualPath::gate_run_record(&GateRunId)` are infallible, because the identity module of `repository_state/path.rs` builds the descendant and the id guarantees one safe component. `VirtualPath::schema(name)` validates its component. Well-known `.jit` files are constants in `declare_well_known_paths!`, and host paths derive through `RepositoryLayout::resolve`. Files under `.git/jit/` are out of scope (DEC-14).

### `system-actor-constants` [implementation-produced] — Engine actors as constants

`Assignee` stores `Cow<'static, str>` parts behind its private fields and gains a `const fn` constructor for engine-owned actors. Associated constants name the list, check, admin, recovery and lease-check actors and the automated gate executor. Parsing and the serialized form are unchanged.

### `compiled-graph-assertions` [plan-fixed] — Typed assertion configuration

Each graph assertion lowers its table at load through a `Raw*` struct, following the existing `RawGateRecency` pattern. It fails with `RuleConfigError::InvalidAssertion { rule, .. }`. Evaluation reads typed fields only. Rule serialization emits the same keys.

### `id-pattern` [implementation-produced] — Compiled pattern

`IdPattern { source, regex }` compares and serializes by source and fails construction on an invalid source. Rule criteria patterns, label coverage and item kinds share it.

## Generated decomposition overview

<!-- jit:breakdown-overview:begin -->
| Key | Title | Type | Outcome | Contracts | Sources | Footprint | Landing | Depends on |
|---|---|---|---|---|---|---|---|---|
| issue-id-types | Validated issue identity type with an alias staging bridge | task | A validated issue-id newtype with a fixed trait set exists beside a temporary String alias. | identity-newtype-convention | REQ-01, REQ-03, DEC-07, DEC-08 | creates 1, touches 1 | — | — |
| graph-node-id-views | Graph node traits expose dependency ids as string views | task | GraphNode and HierarchyNode yield dependency ids as borrowed string views. | — | REQ-01, REQ-17 | touches 5 | — | — |
| graph-node-result-refs | Graph queries return nodes rather than id strings | task | Graph queries return node references, so callers never reparse an id string. | — | REQ-01, REQ-17 | touches 8 | — | graph-node-id-views |
| canonical-unit-test-ids | Library unit tests take canonical issue ids from one fixture | task | Library unit tests take issue ids from one shared fixture that yields canonical ids. | alias-staging-bridge | REQ-01, REQ-17 | touches 29 | — | issue-id-types |
| issue-draft-identity | Issue drafts carry their pre-derived identifier | task | Issue drafts take their id from a dedicated issue-id accessor, so no empty-string issue id exists. | alias-staging-bridge, test-issue-id-fixture | REQ-01, DEC-09 | touches 19 | — | canonical-unit-test-ids |
| prefix-resolution | Id prefixes parse once into an IssueIdPrefix | task | Prefix resolution maps a parsed IssueIdPrefix to an IssueId through the one shared resolver. | alias-staging-bridge, prefix-call-site-bridge | REQ-02, DEC-03, DEC-23 | touches 29 | — | issue-id-types |
| cli-issue-id-boundary | The CLI resolves each issue-id argument once at entry | task | CLI entry resolves each issue-id argument once, before calling the executor. | issue-id-prefix, alias-staging-bridge, prefix-call-site-bridge | REQ-02, REQ-03, REQ-17, DEC-23, DEC-24 | touches 3 | — | prefix-resolution, graph-node-result-refs |
| server-issue-id-boundary | The server resolves each issue-id path parameter once | task | Server routes resolve each issue-id path parameter once, before calling the executor. | issue-id-prefix, alias-staging-bridge, prefix-call-site-bridge | REQ-02, REQ-17, DEC-24 | touches 1 | — | prefix-resolution |
| scoped-issue-id-arguments | Composite arguments carry their issue as a typed id | task | Composite scope arguments hold IssueId, and item references hold IssueIdPrefix. | issue-id-prefix, alias-staging-bridge, prefix-call-site-bridge | REQ-02, REQ-17, DEC-23, DEC-24 | touches 9 | — | cli-issue-id-boundary |
| stage-graph-command-issue-ids | Dependency, graph and template commands carry typed issue ids | task | Graph-editing commands hold issue ids as IssueId and resolve no prefixes. | alias-staging-bridge, issue-id-prefix, prefix-call-site-bridge | REQ-01, REQ-17, DEC-08, DEC-24 | touches 11 | — | cli-issue-id-boundary, server-issue-id-boundary, stage-test-harness-issue-ids |
| stage-gate-command-issue-ids | Gate commands carry typed issue ids | task | Gate commands hold issue ids as IssueId and resolve no prefixes. | alias-staging-bridge, prefix-call-site-bridge | REQ-01, REQ-17, DEC-08, DEC-24 | touches 4 | — | cli-issue-id-boundary, server-issue-id-boundary, stage-test-harness-issue-ids |
| stage-lifecycle-command-issue-ids | Issue lifecycle commands carry typed issue ids | task | Lifecycle commands hold issue ids as IssueId and resolve no prefixes. | alias-staging-bridge, prefix-call-site-bridge | REQ-01, REQ-17, DEC-08, DEC-24 | touches 9 | — | cli-issue-id-boundary, server-issue-id-boundary, stage-test-harness-issue-ids, lease-issue-identity |
| stage-document-command-issue-ids | Document-family commands carry typed issue ids | task | Document-family commands hold issue ids as IssueId and resolve no prefixes. | alias-staging-bridge, prefix-call-site-bridge | REQ-01, REQ-17, DEC-08, DEC-24 | touches 4 | — | scoped-issue-id-arguments, stage-test-harness-issue-ids, server-issue-id-boundary |
| stage-dispatch-issue-ids | Executor core carries typed issue ids | task | The executor core holds issue ids as IssueId and resolves no prefixes. | alias-staging-bridge, prefix-call-site-bridge | REQ-01, REQ-17, DEC-08, DEC-24 | touches 2 | — | cli-issue-id-boundary, server-issue-id-boundary, stage-test-harness-issue-ids |
| stage-repository-state-issue-ids | Repository-state mutations carry typed issue ids | task | Repository-state code holds issue ids as IssueId. | alias-staging-bridge | REQ-01, REQ-17, DEC-08 | touches 4 | — | stage-graph-command-issue-ids, stage-gate-command-issue-ids, stage-lifecycle-command-issue-ids, stage-document-command-issue-ids, stage-dispatch-issue-ids, issue-draft-identity |
| stage-validation-issue-ids | Validation carries typed issue ids | task | Validation code holds issue ids as IssueId. | alias-staging-bridge | REQ-01, REQ-17, DEC-08 | touches 3 | — | stage-graph-command-issue-ids, stage-gate-command-issue-ids, stage-lifecycle-command-issue-ids, stage-document-command-issue-ids, stage-dispatch-issue-ids |
| stage-error-output-issue-ids | Errors and output carry typed issue ids | task | Error and output types hold issue ids as IssueId. | alias-staging-bridge | REQ-01, REQ-17, DEC-08 | touches 3 | — | stage-graph-command-issue-ids, stage-gate-command-issue-ids, stage-lifecycle-command-issue-ids, stage-document-command-issue-ids, stage-dispatch-issue-ids |
| stage-storage-issue-ids | Storage trait and backends take issue ids as IssueId | task | The storage trait, backends and index take issue ids as IssueId. | alias-staging-bridge | REQ-01, REQ-17, DEC-08 | touches 9 | — | stage-repository-state-issue-ids, stage-validation-issue-ids, stage-error-output-issue-ids |
| stage-domain-issue-ids | Domain types carry issue ids as IssueId | task | Domain structs, event bodies and domain functions declare issue ids as IssueId. | alias-staging-bridge | REQ-01, REQ-17, DEC-08 | touches 13 | — | stage-storage-issue-ids, event-issue-accessor |
| stage-test-harness-issue-ids | The shared integration harness carries typed issue ids | task | The shared integration harness returns and takes issue ids as IssueId. | alias-staging-bridge, test-issue-id-fixture | REQ-01, REQ-17, DEC-08 | touches 1 | — | stage-fast-issue-test-issue-ids, stage-fast-docs-templates-test-issue-ids, stage-fast-rules-test-issue-ids, stage-cli-suite-test-issue-ids |
| stage-fast-issue-test-issue-ids | The fast_issue suite carries typed canonical issue ids | task | The fast_issue suite holds issue ids as IssueId with canonical values. | alias-staging-bridge, test-issue-id-fixture | REQ-01, REQ-17, DEC-08 | touches 16 | — | canonical-unit-test-ids |
| stage-fast-docs-templates-test-issue-ids | The fast_docs_templates suite carries typed canonical issue ids | task | The fast_docs_templates suite holds issue ids as IssueId with canonical values. | alias-staging-bridge, test-issue-id-fixture | REQ-01, REQ-17, DEC-08 | touches 15 | — | canonical-unit-test-ids |
| stage-fast-rules-test-issue-ids | The fast_rules suite carries typed canonical issue ids | task | The fast_rules suite holds issue ids as IssueId with canonical values. | alias-staging-bridge, test-issue-id-fixture | REQ-01, REQ-17, DEC-08 | touches 16 | — | canonical-unit-test-ids |
| stage-cli-suite-test-issue-ids | CLI suites that use the library carry typed issue ids | task | Library-facing CLI tests hold issue ids as IssueId. | alias-staging-bridge, test-issue-id-fixture | REQ-01, REQ-17, DEC-08 | touches 12 | — | canonical-unit-test-ids |
| issue-id-flip | IssueId becomes the validated newtype | task | The validated newtype takes the IssueId name, the bridge is gone, and stored ids decode strictly. | issue-identity-newtype, alias-staging-bridge, prefix-call-site-bridge | REQ-01, REQ-17, DEC-07, DEC-08, DEC-15 | touches 5, uncertain | — | stage-domain-issue-ids |
| issue-record-path-storage | Issue record paths come from a VirtualPath constructor | task | VirtualPath::issue_record derives record paths from an IssueId in storage and repository-state code. | virtual-path-constructors, issue-identity-newtype | REQ-01, REQ-07 | touches 6 | — | issue-id-flip |
| issue-record-path-commands | Commands take issue record paths from the VirtualPath constructor | task | Commands and validation take issue record paths from the VirtualPath constructor. | virtual-path-constructors | REQ-07 | touches 10 | — | issue-record-path-storage |
| gate-run-ids | Gate runs are identified by a validated GateRunId | task | Gate runs carry a validated GateRunId from draft to record path. | identity-newtype-convention, virtual-path-constructors | REQ-01, DEC-09 | touches 12 | — | issue-record-path-storage, gate-run-actor |
| gate-key-registry-type | Gate keys are a validated GateKey in the registry and gate runs | task | Gate definitions, gate runs, gate events and the store use a validated GateKey. | identity-newtype-convention | REQ-01, DEC-10 | touches 8 | — | issue-id-flip, domain-profile-id |
| gate-key-issue-fields | Issue gate fields are keyed by GateKey | task | Issue gate fields and gate-key arguments use GateKey. | identity-newtype-convention | REQ-01, REQ-17 | touches 10 | — | gate-key-registry-type |
| server-short-id-field | Server issue responses carry the domain short id | task | Server issue and graph responses carry the domain-derived short id. | issue-identity-newtype | REQ-03, DEC-12 | touches 1 | — | issue-id-flip |
| web-short-id-render | Web UI renders the server-provided short id | task | Web views render the server-provided short id with no id truncation. | — | REQ-03, DEC-12 | touches 4 | — | server-short-id-field |
| event-body-view | Production event matchers read events through a body view | task | Production code matches event variants through Event::body() and reads identity through accessors. | event-envelope | REQ-05, DEC-13 | touches 20 | — | — |
| event-body-unit-test-view | Unit tests match events through the body view | task | Library unit tests match and build events through the body view. | event-body-view | REQ-05, DEC-13 | touches 19 | — | event-body-view |
| event-body-integration-test-view | Integration tests match events through the body view | task | Integration tests match and build events through the body view. | event-body-view | REQ-05, DEC-13 | touches 10 | — | event-body-view |
| event-envelope | Event becomes an envelope around a tagged body | task | Event holds id and timestamp once around a flattened EventBody that drafts build. | event-envelope, event-body-view | REQ-05, DEC-13 | touches 4 | — | event-body-unit-test-view, event-body-integration-test-view |
| event-issue-accessor | Event issue accessor returns an optional IssueId | task | The event issue accessor returns Option<&IssueId> with no empty-string sentinel. | event-envelope, alias-staging-bridge | REQ-04 | touches 11 | — | event-envelope, issue-id-types |
| system-actor-constants | System actors are named Assignee constants | task | Each engine-owned actor is a named Assignee constant. | system-actor-constants | REQ-11 | touches 6 | — | — |
| gate-run-actor | Gate-run actors are Assignee values | task | Gate-run actors are stored and read as Assignee values. | system-actor-constants | REQ-11 | touches 5 | — | system-actor-constants |
| agent-identity-assignee | Resolved agent identity is an Assignee | task | The agent identity is an Assignee from resolution to lease record. | system-actor-constants | REQ-06, REQ-11 | touches 4 | — | system-actor-constants |
| lease-worktree-ids | Lease and worktree ids are dedicated types | task | Lease and worktree ids are validated LeaseId and WorktreeId values. | identity-newtype-convention | REQ-06, DEC-11 | touches 9 | — | agent-identity-assignee, issue-id-types |
| claims-log-canonical-rewrite | This checkout's claims log names issues by full id | task | This checkout's claims log names each lease issue by its full id. | — | REQ-06, DEC-15 | uncertain | — | — |
| lease-issue-identity | Lease records name their issue by IssueId | task | Leases name their issue by IssueId with no short-id tolerance. | alias-staging-bridge, prefix-call-site-bridge | REQ-01, REQ-06, DEC-15 | touches 3 | — | lease-worktree-ids, claims-log-canonical-rewrite, prefix-resolution |
| query-assignee-filter | Query assignee filters parse to an Assignee | task | Query assignee filters hold a parsed Assignee, and the reference shows working filters. | — | REQ-11, REQ-17, DEC-19 | touches 3 | — | — |
| wire-vocabulary-macro | One macro declares a closed wire vocabulary | task | A crate-local macro states each variant's wire spelling once, proven on GateStatus. | wire-vocabulary-macro, exact-token-parsing | REQ-10, REQ-13, DEC-16, DEC-17 | creates 1, touches 4 | — | — |
| event-tag-vocabulary | Event tags declare their wire spelling once | task | Event tags and scopes state their wire spelling once and are typed where compared. | wire-vocabulary-macro, exact-token-parsing | REQ-13, DEC-16 | touches 5 | — | wire-vocabulary-macro, event-envelope |
| artifact-plan-codes | Artifact-plan codes declare their wire spelling once | task | Artifact-plan codes state their wire spelling once. | wire-vocabulary-macro, exact-token-parsing | REQ-13 | touches 1 | — | wire-vocabulary-macro |
| plan-error-derive | Artifact-plan errors use the derive-based error convention | task | PlanError and InventoryError derive Display and Error with unchanged messages. | — | REQ-13 | touches 2 | — | — |
| lifecycle-state-vocabulary | Lifecycle state tokens come from one declaration | task | Lifecycle state tokens come from one macro declaration. | wire-vocabulary-macro, exact-token-parsing | REQ-10, REQ-12, DEC-17 | touches 13 | — | wire-vocabulary-macro |
| priority-vocabulary | Priority tokens come from one declaration | task | Priority tokens come from one macro declaration. | wire-vocabulary-macro, exact-token-parsing | REQ-10, REQ-12, DEC-17 | touches 5 | — | lifecycle-state-vocabulary |
| priority-ord | Priority orders by rank through Ord | task | Priority orders by rank through Ord, with no rank match left. | — | REQ-14 | touches 3 | — | priority-vocabulary |
| validation-strictness-vocabulary | Validation strictness is typed at load | task | Validation strictness decodes once into Strictness. | wire-vocabulary-macro, exact-token-parsing | REQ-10, DEC-17 | touches 5 | — | wire-vocabulary-macro |
| content-format-vocabulary | Content format is typed at load | task | Content format decodes once into ContentFormat with exact tokens. | wire-vocabulary-macro, exact-token-parsing | REQ-10, DEC-17 | touches 5 | — | validation-strictness-vocabulary |
| kind-scope-vocabulary | Item-kind scope has one enum | task | Item-kind scope has one domain enum with serde. | wire-vocabulary-macro, exact-token-parsing | REQ-10 | touches 3 | — | wire-vocabulary-macro |
| transform-kind-vocabulary | Template transform kind is typed at load | task | Template transform kinds decode once into TransformKind. | wire-vocabulary-macro, exact-token-parsing | REQ-10 | touches 4 | — | wire-vocabulary-macro |
| projection-mode-style-vocabulary | Projection mode and style reach output typed | task | Projection render reports carry mode and style as enums. | wire-vocabulary-macro, exact-token-parsing | REQ-10 | touches 2 | — | wire-vocabulary-macro |
| gate-finding-vocabulary | Built-in gate findings use closed severity and disposition enums | task | Engine-emitted gate findings use closed severity, disposition and origin enums. | wire-vocabulary-macro, exact-token-parsing | REQ-10, DEC-18 | touches 2 | — | wire-vocabulary-macro |
| repository-action-owner | Repository action owners are one enum | task | Repository action owners are one enum, matched by variant. | wire-vocabulary-macro, exact-token-parsing | REQ-10 | touches 10 | — | wire-vocabulary-macro |
| projection-typed-fields | Validation projections hold typed state and priority | task | Validation projections hold typed State and Priority. | exact-token-parsing | REQ-12, DEC-20 | touches 1 | — | priority-vocabulary |
| rule-severity-scope-vocabulary | Rule severity and scope tokens come from one declaration | task | Rule severity and scope state their spelling once. | wire-vocabulary-macro, exact-token-parsing | REQ-10, DEC-21 | touches 2 | — | lifecycle-state-vocabulary |
| type-hierarchy-kind-vocabulary | Type-hierarchy and label-uniqueness rule options are typed | task | Type-hierarchy kind and label-uniqueness scope decode once into enums. | wire-vocabulary-macro, exact-token-parsing | REQ-10, REQ-15, DEC-21 | touches 3 | — | rule-severity-scope-vocabulary |
| gate-stage-mode-vocabulary | Gate stage and mode tokens come from one declaration | task | Gate stage and mode state their spelling once. | wire-vocabulary-macro, exact-token-parsing | REQ-10, DEC-21 | touches 1 | — | wire-vocabulary-macro |
| error-code-vocabulary | Error codes come from one declaration | task | Error codes state their spelling once. | wire-vocabulary-macro, exact-token-parsing | REQ-10, DEC-21 | touches 1 | — | wire-vocabulary-macro |
| divergence-vocabulary | Store divergence tokens come from one declaration | task | Store divergence vocabularies state their spelling once. | wire-vocabulary-macro, exact-token-parsing | REQ-10, DEC-21 | touches 1 | — | wire-vocabulary-macro |
| debug-token-renders | Human output renders vocabulary tokens, not Debug names | task | Human output prints vocabulary wire tokens. | wire-vocabulary-macro, exact-token-parsing | REQ-10, REQ-17 | touches 3 | — | priority-vocabulary |
| compiled-id-pattern | Rule patterns compile at declaration load | task | Criteria patterns compile once at load through one IdPattern type. | compiled-graph-assertions | REQ-15 | touches 3 | — | — |
| item-kind-id-pattern | Item kinds hold their id pattern as IdPattern | task | Item kinds hold their id pattern as the shared IdPattern. | id-pattern | REQ-15 | touches 1 | — | compiled-id-pattern |
| typed-label-reference | Label-reference assertions are typed at load | task | Label-reference assertions decode into a typed configuration at load. | compiled-graph-assertions | REQ-15 | touches 4 | — | type-hierarchy-kind-vocabulary, compiled-id-pattern |
| typed-dependency-shape | Dependency-shape assertions are typed at load | task | Dependency-shape assertions decode into a typed configuration with a load-parsed selector. | compiled-graph-assertions | REQ-15 | touches 3 | — | typed-label-reference |
| typed-label-coverage | Label-coverage assertions are typed and compiled at load | task | Label-coverage assertions decode into a typed configuration with a compiled pattern. | compiled-graph-assertions, id-pattern | REQ-15 | touches 5 | — | typed-dependency-shape |
| hierarchy-config-constructor | HierarchyConfig has one fallible constructor from declarations | task | One fallible constructor builds HierarchyConfig, and the three sites route through it. | — | REQ-16, DEC-22 | touches 4 | — | — |
| hierarchy-config-callers | Hierarchy callers propagate the constructor's error | task | Hierarchy callers propagate the one constructor's error. | — | REQ-16, DEC-22 | touches 11 | — | hierarchy-config-constructor |
| schema-path-constructor | Schema paths come from a VirtualPath constructor | task | Schema file paths come from one validated VirtualPath constructor. | virtual-path-constructors | REQ-07 | touches 7 | — | — |
| well-known-paths-repository-state | Repository-state code names well-known files through VirtualPath constants | task | Repository-state code names well-known files only through VirtualPath constants. | virtual-path-constructors | REQ-07, DEC-14 | touches 6 | — | schema-path-constructor |
| well-known-paths-commands | Commands name well-known files through VirtualPath constants | task | Commands name well-known files only through VirtualPath constants. | virtual-path-constructors | REQ-07, DEC-14 | touches 9 | — | well-known-paths-repository-state |
| well-known-paths-storage | Storage and configuration code name well-known files through VirtualPath constants | task | Storage and loaders name well-known files only through VirtualPath constants. | virtual-path-constructors | REQ-07, DEC-14 | touches 13 | — | well-known-paths-commands |
| control-file-path-constants | Worktree and server files under .jit have VirtualPath constants | task | Worktree identity and server files under .jit are named through VirtualPath constants. | virtual-path-constructors | REQ-07, DEC-14 | touches 4 | — | well-known-paths-storage |
| reserved-target-predicate | One VirtualPath predicate recognises reserved application targets | task | Reserved application targets are recognised by one VirtualPath predicate after classification. | virtual-path-constructors | REQ-08 | touches 3 | — | — |
| domain-profile-id | ProfileId is defined in the domain layer | task | ProfileId and its kebab predicate live in domain::identity. | identity-newtype-convention | REQ-09, DEC-06 | touches 14 | — | issue-id-types |
| typed-profile-dependency-resolution | Profile dependency resolution compares ProfileId values | task | Profile dependency resolution and the package graph compare ProfileId values. | — | REQ-09, DEC-06 | touches 2 | — | domain-profile-id |
| issue-identity | Issue identity is one validated type end to end | story | Issue identity, prefixes, gate-run ids, gate keys and record paths are typed end to end, with the staging bridge removed. | — | REQ-01, REQ-02, REQ-03, REQ-07, REQ-17 | — | — | issue-record-path-commands, gate-run-ids, gate-key-issue-fields, web-short-id-render |
| event-records | Events are an envelope around a tagged body | story | Events hold id and timestamp once around a tagged body, with a typed optional issue accessor. | — | REQ-04, REQ-05 | — | — | event-issue-accessor |
| actor-identity | Actors and leases carry typed identities | story | System actors, gate-run actors, query filters and lease records carry Assignee and dedicated id types. | — | REQ-06, REQ-11 | — | — | gate-run-actor, lease-issue-identity, query-assignee-filter |
| wire-vocabularies | Closed vocabularies state their wire spelling once | story | Each closed vocabulary is one macro-declared enum with exact-token parsing and no restated token. | — | REQ-10, REQ-12, REQ-13, REQ-14 | — | — | event-tag-vocabulary, artifact-plan-codes, plan-error-derive, priority-ord, content-format-vocabulary, kind-scope-vocabulary, transform-kind-vocabulary, projection-mode-style-vocabulary, gate-finding-vocabulary, repository-action-owner, projection-typed-fields, type-hierarchy-kind-vocabulary, gate-stage-mode-vocabulary, error-code-vocabulary, divergence-vocabulary, debug-token-renders |
| typed-declarations | Declarations are typed and compiled at load | story | Graph assertions and the type hierarchy are typed and validated when declarations load. | — | REQ-15, REQ-16 | — | — | item-kind-id-pattern, typed-label-coverage, hierarchy-config-callers |
| repository-paths | Repository paths and profile identity are typed | story | Well-known files, schema paths and reserved targets go through VirtualPath, and ProfileId lives in the domain. | — | REQ-07, REQ-08, REQ-09 | — | — | control-file-path-constants, reserved-target-predicate, typed-profile-dependency-resolution |

```mermaid
flowchart LR
    N0["issue-id-types: Validated issue identity type with an alias staging bridge"]
    N1["graph-node-id-views: Graph node traits expose dependency ids as string views"]
    N2["graph-node-result-refs: Graph queries return nodes rather than id strings"]
    N3["canonical-unit-test-ids: Library unit tests take canonical issue ids from one fixture"]
    N4["issue-draft-identity: Issue drafts carry their pre-derived identifier"]
    N5["prefix-resolution: Id prefixes parse once into an IssueIdPrefix"]
    N6["cli-issue-id-boundary: The CLI resolves each issue-id argument once at entry"]
    N7["server-issue-id-boundary: The server resolves each issue-id path parameter once"]
    N8["scoped-issue-id-arguments: Composite arguments carry their issue as a typed id"]
    N9["stage-graph-command-issue-ids: Dependency, graph and template commands carry typed issue ids"]
    N10["stage-gate-command-issue-ids: Gate commands carry typed issue ids"]
    N11["stage-lifecycle-command-issue-ids: Issue lifecycle commands carry typed issue ids"]
    N12["stage-document-command-issue-ids: Document-family commands carry typed issue ids"]
    N13["stage-dispatch-issue-ids: Executor core carries typed issue ids"]
    N14["stage-repository-state-issue-ids: Repository-state mutations carry typed issue ids"]
    N15["stage-validation-issue-ids: Validation carries typed issue ids"]
    N16["stage-error-output-issue-ids: Errors and output carry typed issue ids"]
    N17["stage-storage-issue-ids: Storage trait and backends take issue ids as IssueId"]
    N18["stage-domain-issue-ids: Domain types carry issue ids as IssueId"]
    N19["stage-test-harness-issue-ids: The shared integration harness carries typed issue ids"]
    N20["stage-fast-issue-test-issue-ids: The fast_issue suite carries typed canonical issue ids"]
    N21["stage-fast-docs-templates-test-issue-ids: The fast_docs_templates suite carries typed canonical issue ids"]
    N22["stage-fast-rules-test-issue-ids: The fast_rules suite carries typed canonical issue ids"]
    N23["stage-cli-suite-test-issue-ids: CLI suites that use the library carry typed issue ids"]
    N24["issue-id-flip: IssueId becomes the validated newtype"]
    N25["issue-record-path-storage: Issue record paths come from a VirtualPath constructor"]
    N26["issue-record-path-commands: Commands take issue record paths from the VirtualPath constructor"]
    N27["gate-run-ids: Gate runs are identified by a validated GateRunId"]
    N28["gate-key-registry-type: Gate keys are a validated GateKey in the registry and gate runs"]
    N29["gate-key-issue-fields: Issue gate fields are keyed by GateKey"]
    N30["server-short-id-field: Server issue responses carry the domain short id"]
    N31["web-short-id-render: Web UI renders the server-provided short id"]
    N32["event-body-view: Production event matchers read events through a body view"]
    N33["event-body-unit-test-view: Unit tests match events through the body view"]
    N34["event-body-integration-test-view: Integration tests match events through the body view"]
    N35["event-envelope: Event becomes an envelope around a tagged body"]
    N36["event-issue-accessor: Event issue accessor returns an optional IssueId"]
    N37["system-actor-constants: System actors are named Assignee constants"]
    N38["gate-run-actor: Gate-run actors are Assignee values"]
    N39["agent-identity-assignee: Resolved agent identity is an Assignee"]
    N40["lease-worktree-ids: Lease and worktree ids are dedicated types"]
    N41["claims-log-canonical-rewrite: This checkout's claims log names issues by full id"]
    N42["lease-issue-identity: Lease records name their issue by IssueId"]
    N43["query-assignee-filter: Query assignee filters parse to an Assignee"]
    N44["wire-vocabulary-macro: One macro declares a closed wire vocabulary"]
    N45["event-tag-vocabulary: Event tags declare their wire spelling once"]
    N46["artifact-plan-codes: Artifact-plan codes declare their wire spelling once"]
    N47["plan-error-derive: Artifact-plan errors use the derive-based error convention"]
    N48["lifecycle-state-vocabulary: Lifecycle state tokens come from one declaration"]
    N49["priority-vocabulary: Priority tokens come from one declaration"]
    N50["priority-ord: Priority orders by rank through Ord"]
    N51["validation-strictness-vocabulary: Validation strictness is typed at load"]
    N52["content-format-vocabulary: Content format is typed at load"]
    N53["kind-scope-vocabulary: Item-kind scope has one enum"]
    N54["transform-kind-vocabulary: Template transform kind is typed at load"]
    N55["projection-mode-style-vocabulary: Projection mode and style reach output typed"]
    N56["gate-finding-vocabulary: Built-in gate findings use closed severity and disposition enums"]
    N57["repository-action-owner: Repository action owners are one enum"]
    N58["projection-typed-fields: Validation projections hold typed state and priority"]
    N59["rule-severity-scope-vocabulary: Rule severity and scope tokens come from one declaration"]
    N60["type-hierarchy-kind-vocabulary: Type-hierarchy and label-uniqueness rule options are typed"]
    N61["gate-stage-mode-vocabulary: Gate stage and mode tokens come from one declaration"]
    N62["error-code-vocabulary: Error codes come from one declaration"]
    N63["divergence-vocabulary: Store divergence tokens come from one declaration"]
    N64["debug-token-renders: Human output renders vocabulary tokens, not Debug names"]
    N65["compiled-id-pattern: Rule patterns compile at declaration load"]
    N66["item-kind-id-pattern: Item kinds hold their id pattern as IdPattern"]
    N67["typed-label-reference: Label-reference assertions are typed at load"]
    N68["typed-dependency-shape: Dependency-shape assertions are typed at load"]
    N69["typed-label-coverage: Label-coverage assertions are typed and compiled at load"]
    N70["hierarchy-config-constructor: HierarchyConfig has one fallible constructor from declarations"]
    N71["hierarchy-config-callers: Hierarchy callers propagate the constructor's error"]
    N72["schema-path-constructor: Schema paths come from a VirtualPath constructor"]
    N73["well-known-paths-repository-state: Repository-state code names well-known files through VirtualPath constants"]
    N74["well-known-paths-commands: Commands name well-known files through VirtualPath constants"]
    N75["well-known-paths-storage: Storage and configuration code name well-known files through VirtualPath constants"]
    N76["control-file-path-constants: Worktree and server files under .jit have VirtualPath constants"]
    N77["reserved-target-predicate: One VirtualPath predicate recognises reserved application targets"]
    N78["domain-profile-id: ProfileId is defined in the domain layer"]
    N79["typed-profile-dependency-resolution: Profile dependency resolution compares ProfileId values"]
    N80["issue-identity: Issue identity is one validated type end to end"]
    N81["event-records: Events are an envelope around a tagged body"]
    N82["actor-identity: Actors and leases carry typed identities"]
    N83["wire-vocabularies: Closed vocabularies state their wire spelling once"]
    N84["typed-declarations: Declarations are typed and compiled at load"]
    N85["repository-paths: Repository paths and profile identity are typed"]
    N1 --> N2
    N0 --> N3
    N3 --> N4
    N0 --> N5
    N5 --> N6
    N2 --> N6
    N5 --> N7
    N6 --> N8
    N6 --> N9
    N7 --> N9
    N19 --> N9
    N6 --> N10
    N7 --> N10
    N19 --> N10
    N6 --> N11
    N7 --> N11
    N19 --> N11
    N42 --> N11
    N8 --> N12
    N19 --> N12
    N7 --> N12
    N6 --> N13
    N7 --> N13
    N19 --> N13
    N9 --> N14
    N10 --> N14
    N11 --> N14
    N12 --> N14
    N13 --> N14
    N4 --> N14
    N9 --> N15
    N10 --> N15
    N11 --> N15
    N12 --> N15
    N13 --> N15
    N9 --> N16
    N10 --> N16
    N11 --> N16
    N12 --> N16
    N13 --> N16
    N14 --> N17
    N15 --> N17
    N16 --> N17
    N17 --> N18
    N36 --> N18
    N20 --> N19
    N21 --> N19
    N22 --> N19
    N23 --> N19
    N3 --> N20
    N3 --> N21
    N3 --> N22
    N3 --> N23
    N18 --> N24
    N24 --> N25
    N25 --> N26
    N25 --> N27
    N38 --> N27
    N24 --> N28
    N78 --> N28
    N28 --> N29
    N24 --> N30
    N30 --> N31
    N32 --> N33
    N32 --> N34
    N33 --> N35
    N34 --> N35
    N35 --> N36
    N0 --> N36
    N37 --> N38
    N37 --> N39
    N39 --> N40
    N0 --> N40
    N40 --> N42
    N41 --> N42
    N5 --> N42
    N44 --> N45
    N35 --> N45
    N44 --> N46
    N44 --> N48
    N48 --> N49
    N49 --> N50
    N44 --> N51
    N51 --> N52
    N44 --> N53
    N44 --> N54
    N44 --> N55
    N44 --> N56
    N44 --> N57
    N49 --> N58
    N48 --> N59
    N59 --> N60
    N44 --> N61
    N44 --> N62
    N44 --> N63
    N49 --> N64
    N65 --> N66
    N60 --> N67
    N65 --> N67
    N67 --> N68
    N68 --> N69
    N70 --> N71
    N72 --> N73
    N73 --> N74
    N74 --> N75
    N75 --> N76
    N0 --> N78
    N78 --> N79
    N26 --> N80
    N27 --> N80
    N29 --> N80
    N31 --> N80
    N36 --> N81
    N38 --> N82
    N42 --> N82
    N43 --> N82
    N45 --> N83
    N46 --> N83
    N47 --> N83
    N50 --> N83
    N52 --> N83
    N53 --> N83
    N54 --> N83
    N55 --> N83
    N56 --> N83
    N57 --> N83
    N58 --> N83
    N60 --> N83
    N61 --> N83
    N62 --> N83
    N63 --> N83
    N64 --> N83
    N66 --> N84
    N69 --> N84
    N71 --> N84
    N76 --> N85
    N77 --> N85
    N79 --> N85
```
<!-- jit:breakdown-overview:end -->

## Material risks and owner decisions

| Risk / decision | Resolution and rationale |
|---|---|
| DEC-01 Cutover boundary | Chosen: 842b0ac6 lands first and deletes the items it lists. This epic types only what survives. Rejected: typing items that are about to be deleted. |
| DEC-02 842b0ac6 edge | Chosen: remove the edge from 8aa53c43, because it is consumed by implementation rather than needed for planning. External dependency 842b0ac6 → manifest keys `issue-id-types`, `graph-node-id-views`, `event-body-view`, `system-actor-constants`, `query-assignee-filter`, `wire-vocabulary-macro`, `plan-error-derive`, `compiled-id-pattern`, `hierarchy-config-constructor`, `schema-path-constructor`, `reserved-target-predicate`, `claims-log-canonical-rewrite` (every root; re-home at breakdown). Rejected: keeping it on the planning node. |
| DEC-03 16d63038 | Chosen: external dependency 16d63038 → manifest key `prefix-resolution`. The boundary entries, the command staging entries and the flip reach it transitively, so no further edge is needed (a direct one would be transitively redundant). Rejected: absorbing resolver behaviour here. |
| DEC-04 aa423d02 | Chosen: external dependency aa423d02 → manifest keys `priority-ord` and `stage-lifecycle-command-issue-ids`, which both edit `commands/issue.rs` near `claim_next`. Rejected: rebasing over it. |
| DEC-05 Work owned elsewhere | Chosen: no work here for strategic resolution (f368ce3d), CLI filter groups (df129d9e), the event-log parser and lease sequences (f8a6b298), or evaluation algorithms (16c53cc2). 7d1bfe89 was merged at `39ac419f0`, so `query-assignee-filter` needs no edge. Rejected: edges to downstream epics, which already depend on 2195c98f. |
| DEC-06 Profile scope | Chosen: move `ProfileId` to the domain, and type dependency resolution and the package graph. `ProfilePackageId`, semver versions and record comparisons stay with 3d9a0ce3. Rejected: retyping code that D-8 deletes. No edge to 3d9a0ce3. |
| DEC-07 IssueId representation | Chosen: a canonical-string newtype with strict decoding. Rejected: a `Uuid` inner, which adds an allocation at every string boundary and a second canonical form. |
| DEC-08 Staging | Chosen: alias staging, verified by criteria in each family and caught by the compiler at the flip. Rejected: a big-bang flip spread over non-green commits (293 errors). Risk: staging steps are not compiler-checked, and the flip's residue is uncertain, which its footprint declares. |
| DEC-09 Draft identity | Chosen: issue and gate-run drafts carry the id from `identifier_at`. Rejected: id-less draft types (more types) and a nil-UUID placeholder (a sentinel). |
| DEC-10 Gate-key grammar | Verified: the registry enforces no grammar (`declarations/gates.rs:159`; only the duplicate check at `:281`). Chosen: lowercase-kebab through the shared domain predicate, which every key in this repository satisfies. Rejected: non-empty only, which leaves keys unvalidated. |
| DEC-11 WorktreeId | Chosen: a non-empty token without whitespace, built by both fallbacks. Rejected: removing the fallbacks. Risk: a worktree root path that contains whitespace now errors instead of yielding an id. |
| DEC-12 Web short ids | Chosen: the server serves `short_id`, and the web renders it (two entries). Rejected: declaring the web a boundary outside REQ-03. |
| DEC-13 Event naming | Chosen: `Event` is the envelope and `EventBody` the tagged body, per REQ-05. Rejected: an `EventRecord` envelope. The rename is split green through `event-body-view`. |
| DEC-14 REQ-07 scope | Chosen: `.jit`-resident files get constants, including `worktree.json`, `server.pid.json` and `server.log`. Rejected: covering `.git/jit/` control-plane files. |
| DEC-15 Local lease data | Chosen: short-id lease records are rejected once ids are validated. The machine-local claims-log rewrite is its own terminal, `claims-log-canonical-rewrite`, which lands before the tolerance is deleted and therefore before the flip; current code already accepts full-id records, so it lands green alone. The flip carries no migration. Rejected: a compatibility reader. Repository `.jit/` data keeps its serialized shapes. |
| DEC-16 Vocabulary macro | Chosen: a crate-local `macro_rules!`, owned by one entry and consumed by every vocabulary entry. Rejected: strum, which adds two proc-macro crates and a second statement of the rename rule. |
| DEC-17 Exact tokens | Chosen: exact canonical tokens only. Rejected: case folding and the `md` alias. |
| DEC-18 Gate-finding vocabulary | Chosen: closed enums for engine-emitted findings, with the wire fields kept open. Rejected: an open enum with an `Other` fallback. |
| DEC-19 Query assignee | Chosen: a strict `Assignee`, with the `NOT assignee:*` example fixed in the same entry. Rejected: keeping silent non-matches. |
| DEC-20 Projection schema | Chosen: accept the change from a string schema to an enum schema. Rejected: keeping `String` fields. |
| DEC-21 Unnamed token twins | Chosen: rule severity and scope, gate stage and mode, `ErrorCode`, the divergence enums and the type-hierarchy kind, each in a bounded macro entry. Rejected: leaving them out, which is a convergence defect. |
| DEC-22 HierarchyConfig | Chosen: one fallible constructor at three production sites. The silent fallback is removed, and callers propagate the error. Rejected: also validating at declaration load (optional follow-up). |
| DEC-23 Id-input error classes | Chosen: follow the landed 16d63038 contract, which is authoritative. A prefix that normalises below the minimum length gets the invalid-prefix error before any record is read, and well-formed-length input outside the id alphabet or matching nothing keeps not-found and exit code 3. Rejected: a new invalid-argument class, which is a contract change no criterion asks for. |
| DEC-24 Prefix boundary | Chosen: executor and internal command methods take `IssueId`, and `IssueIdPrefix` is parsed and resolved only at the CLI and server entry points, each owned by one entry. Composite arguments (archive, document and snapshot scopes) resolve there too. The two in-executor cases are dependency removal over the issue's own dependency list and item-address segments decoded from documents. Rejected: prefix-typed executor methods, which re-resolve full ids and churn about 160 test call sites twice. |
| Shared-file contention | `domain/types.rs`, `main.rs`, `commands/mod.rs`, `declarations/rules.rs`, `validation/graph.rs`, `test_utils.rs` and `crates/server/src/routes.rs` are edited by several entries. Edges serialize every same-region rewrite the review found: the event chain before domain staging, the gate-run actor before gate-run ids, the pattern type before the assertion chain, and the well-known-path chain. The footprint advisories list the overlaps that remain; workers rebase over them. |

## Investigation sources

- [Investigation](investigation.md) holds the consumer and file inventories, the experiment evidence (293-error flip, envelope probe) and the split points. Its criterion numbers are pre-2abebdd7c, and its header maps them.
