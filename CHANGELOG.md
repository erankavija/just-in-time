# Changelog

All notable changes to Just-In-Time (JIT) are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Removed

- **Build provenance, the stale-binary guard, and the disk-backed gate
  `TMPDIR`.** `crates/jit/build.rs` stamped a commit, a dirty flag, and a build
  timestamp into the binary; `jit --version` and `jit version --json` reported
  them; and a guard refused gate evaluation with exit code `10` and error code
  `STALE_BINARY` when the installed binary predated the tree under review.
  Proving that contract took three `#[ignore]`d tests that each compiled a whole
  crate into a fresh temporary target directory, which is why `scripts/cargo-ci.sh`
  moved `TMPDIR` off tmpfs onto a disk-backed cache directory for its entire run.
  Measured on one host at a single commit, that override cost the whole
  filesystem-heavy suite 104,285 ms against 18,341 ms on tmpfs — a 5.8x tax paid
  by every other test in the gate — while the suite itself took 116,971 ms of a
  192,994 ms gate. The guard's own remedy was a manual reinstall, which a stale
  binary made visible anyway. All of it is gone: the build script reads nothing
  ambient, `jit version` reports package, version, profile, and target,
  `scripts/install-jit.sh` is `cargo install --path crates/jit`, the
  `provenance` gate step and the `TMPDIR` override are removed, and exit code
  `10` no longer means a refusal on binary age.

- **The per-merge build guard.** `scripts/verify-commit-builds.sh` extracted a
  named commit with `git archive` and ran `cargo build --workspace` against it,
  and the worktree dispatch protocol ran it after every merge. `cargo build`
  compiles neither test targets nor dev-dependencies, so the guard passed a
  merge that broke only test code — observed here, where one branch changed a
  function's signature while another added a `#[cfg(test)]` caller of the old
  form — and passed a merge that compiled and failed when its tests ran; its
  implausibly short "cold build" was that blind spot showing. The protocol now
  runs the project's build-and-test gate on the merged tree, work that gate
  already performs, and states what a gate run does and does not establish about
  a merge commit. `scripts/cargo-ci-selftest.sh` seeds clean merges that break in
  each of those ways and asserts `scripts/cargo-ci.sh` reports them failing, so
  the replacement cannot silently go vacuous: substituting the removed guard's
  build-only command makes the self-test fail.

- **The configuration the binary wrote for a new repository.** `jit init`
  rendered a whole `config.toml` into every repository it created: a four-level
  type hierarchy, seven label namespaces, six item kinds, validation defaults,
  a development-area classification, and the commented guidance around them.
  None of it was asked for, and all of it is now declared by the `jit-default`
  profile package instead, so a bare initialization writes only what the engine
  derives from the repository — the schema version and the project name slugged
  from the directory — beside the empty gate and invariant registries, the event
  log, and the index. The rule set it derives shrinks with the registry it
  derives from: a repository declaring no namespaces and no type hierarchy
  receives the label grammar alone and the one schema projection that rule
  references. Such a repository is usable — it validates and accepts issues —
  and obtains a vocabulary by applying a package that declares one:
  `jit init --profile jit-default --from <path>` produces the configuration a
  plain `jit init` used to write.

- **The four named hierarchy presets, the `--hierarchy-template` flag, and
  `jit config list-templates`.** The presets (`default`, `extended`, `agile`,
  `minimal`) were a bundle of type names selected by name; the flag and the
  listing command existed only to select and enumerate them. All three are gone
  rather than left answering with an empty set, and `jit init --json` no longer
  reports a `hierarchy_template` field. An adopter who chose a taxonomy by
  asking the binary which ones existed now reads a package directory's
  `manifest.toml` and applies the one they want.

- **The compiled documentation-area classification.** `SHIPPED_DOCUMENTATION_POLICY`
  named this project's own development areas and supplied them to any repository
  whose `[documentation]` table omitted a key. An absent key now declares
  nothing, so no area name reaches a repository that never named one, and the
  generated `[documentation]` regions in `docs/reference/configuration.md` and
  `docs/reference/example-config.toml` are rendered from the `jit-default`
  package's own contributions.

- **The compiled-in workflow package and gate presets.** The binary carried the
  `jit-dogfood` workflow package and three gate presets derived from its plan
  template, and a profile record could identify a package compiled into the
  binary; the dependency resolver could use those bytes as a last resort. Those
  routes are removed. Profile packages are read from directories inside the
  repository, each applied record retains its worktree-relative location, and a
  dependency is resolved beside the package that declares it before the
  repository's own applied-profile records are consulted. The native archive
  supplies `packages/jit-default/` and `packages/jit-dogfood/`, so an adopter
  applies the package from the extracted directory, while gate names not
  supplied by a profile resolve from the repository's own `.jit/gates.toml`.

### Fixed

- **A concurrency property decides on coordination rather than on how busy the
  host was.** The claim coordinator's concurrent-acquisition tests spawned a
  thread per issue and asserted every one was granted, but a thread reached the
  coordinator only through a lock wait measured in wall-clock seconds. A thread
  that was still queued when its wait expired failed a property about mutual
  exclusion for want of time, so the authoritative quality gate returned a
  different verdict for the same tree depending on what else the machine was
  doing — enough that the gate script capped its test threads below the core
  count to narrow the window, and named that proptest as the reason it
  serializes builds host-wide. An expired wait is now its own error type,
  `storage::lock::LockTimeout`, distinct from any answer the locked operation
  gave, and every concurrent-acquisition test — the two properties and the three
  example-based cases — treats it as "not there yet" and asks again. Load
  changes how many times a caller asks and nothing about what it is told, so the
  cap and both stated reasons for it are gone; the build lock keeps the reasons
  that survive, CPU oversubscription and peak RAM. The retry is bounded on the
  claimants' own progress rather than on a clock: it fails loudly when no
  claimant at all has been answered for thirty seconds, which is a stuck lock
  rather than a lost race. Two tests hold the separation deterministically by
  holding the claims lock until every claimant has been refused it, the
  condition an arbitrarily busy host produces on its own, and a third observes
  that a caller still queued for a held lock reports an expired wait and is
  granted the same claim once the lock is free. A survey of every remaining
  assertion in the gate's suite whose outcome could turn on host load, and the
  result recorded for each, is in `dev/active/57675b68/`.

- **The shutdown drain property is proven against a server that breaks it.** The
  real-process graceful-shutdown case asserts that a connection stuck mid message
  received the whole drain deadline before the server retired it, reading the
  duration the server measured across its own drain. That comparison holds
  whatever the host is doing, and it held equally against a server that never
  drained at all, because nothing exercised the failure. `jit-server` built for
  its own tests now resolves the drain deadline through a fault-injection seam,
  and one case spawns that binary with no deadline at all: the same fixture, the
  same signal, the same forced close, and the same assertion, which must reject
  the log it produces. Neutralising either half — the seed or the comparison —
  makes that case fail. The seam is compiled only when the crate's `test-support`
  feature is on, which shipped builds resolve without.

### Added

- **`jit profile pack` and `jit profile add` move a package as one verifiable
  file.** Sharing a package meant copying a directory, with nothing stating what
  was copied and nothing confirming it arrived whole; a truncated copy became a
  package that decoded and was wrong. `jit profile pack --source <DIR> --output
  <FILE>` now writes one uncompressed tar carrying one entry per package file
  beside its id, version, and the content digest that is already the package's
  identity, and `jit profile add --archive <FILE> --destination <DIR>`
  recomputes that identity from the extracted content and refuses an archive
  whose content does not reproduce it. Packing the same package twice produces
  byte-identical output, so an archive can be compared or checksummed without
  re-reading the package. The archive carries no directory entries — the
  publication builds the tree from the file paths — which is what makes what an
  archive holds a function of a package's file count rather than of how deeply
  its paths nest, so the bounds the read enforces hold for every package the
  package model accepts. The digest travels inside the archive: it establishes integrity, not
  origin. An arriving archive is read as untrusted input — an entry naming an
  absolute path or a parent-directory traversal, an entry that is not a regular
  file or a directory, an entry carrying an unexpected mode, and content past
  the package file or byte budget are each refused, the budgets against running
  counts while the archive is being read rather than after its bytes have
  landed. Every mode is compared and never adopted: a package file's against the
  mode its manifest declaration implies, and the metadata entry's and any
  directory entry's against the mode the archive format defines. Extraction produces a value in memory and the
  finished tree is published through the same recoverable transaction a capture
  uses, so a refused or interrupted add leaves no partially extracted package
  and an occupied destination is never published over.

- **`jit profile capture` republishes a package tree from the repository files
  its manifest declares.** Producing a package tree was a repository-local
  render compiled only for this crate's tests, hardcoded to one package path and
  one entry-point script, so no adopter could author a package from a configured
  repository, and profile-owned content edited in place could not be folded back
  into the package that owns it — an edit to a packaged skill or gate script had
  to be made twice, and a missed second edit was drift a guard could only report
  after the fact. `jit profile capture --source <DIR>
  --destination <DIR>` now serves any package directory and any destination its
  arguments name. It draws each asset declared under the `assets/live/` package
  source prefix from the repository file its declaration targets and every other
  declared source from the package directory itself, so a capture after an
  in-place edit publishes a tree whose package identity reflects that edit and
  re-applying it reconciles the repository. A declared target that is a symbolic
  link, resolves outside the worktree, or carries executable permission its
  declaration did not is refused before anything is published, and the captured
  content is validated as a package before publication. Publication is whole and
  recoverable: the destination ends up holding exactly the manifest, the
  declared asset sources, and the declared region sources, and it rides the same
  transaction every other repository mutation does, so a failure leaves an
  occupied destination as it was rather than losing it. A capture that changes
  nothing publishes nothing.

- **A profile package is applied together with the packages it depends on.** A
  manifest could declare a dependency, and nothing read it: a package carrying
  only its delta over another applied against a repository missing the
  vocabulary it extends. Applying a package now resolves everything it declares
  a dependency on, transitively, and applies each before the package that
  declares it, so a delta package and a self-contained one produce the same
  repository. A dependency is looked for beside the declaring package's own
  directory, under the dependency's own id, before this repository's record
  answers — so one obtained directory of packages applies as a set without every
  member being named. Each applied package writes its own
  provenance record and appends its own audit event, a package two others depend
  on is applied once, and re-applying a set that is already applied reports no
  work anywhere in it. Declared dependencies that close a cycle are rejected
  naming the cycle, and a dependency that resolves through no route fails the
  application naming the package that declared it beside the one that could not
  be found; both refusals are raised over the whole set before any of it is
  applied. `jit profile apply --json` therefore answers with the standard
  count-wrapped envelope, one result per applied package, and `jit validate`
  reporting a recorded package it cannot obtain states which recorded package
  declared a dependency on it, where one did.

- **One native download now carries the captured `jit-dogfood` workflow package
  for offline profile application.** An adopter could previously download the
  native archive but still needed a second source to apply the workflow package
  offline; the archive now carries the captured package under
  `packages/jit-dogfood/`. The `jit` and `jit-server` binaries and both license
  texts remain at the extraction root, so the documented install step is
  unchanged. The published asset set and checksum file are unchanged because
  the package rides inside the archive they already cover.

- **The native archive now carries the default vocabulary beside the workflow
  package.** The extracted `jit-dogfood` package can therefore resolve and apply
  its `jit-default` dependency from `packages/jit-default/`, so one native
  download contains the complete profile set needed for offline application.
  Both package directories remain inside the existing native archive, so the
  published asset set and checksum coverage are unchanged.

- **A gate verdict is reused when its declared inputs are unchanged.** A quality
  gate runs once per issue, so several issues sitting on one repository state
  each paid for a whole-tree checker to re-derive an identical verdict; nothing
  in a run record supported avoiding it, because a gate pipeline dirties the
  tree by running and consecutive runs over one unchanged source state
  therefore disagreed about whether the tree was clean. A gate declaration now
  carries the repository files its checker reads, as a `[gates.inputs]` table of
  roots with glob exclusions — the same root-and-exclusion shape a profile
  manifest states its live sources in, and the same one the binary's own
  build-input inventory is now expressed as. Evaluating a gate that declares
  inputs digests their content first, over every file beneath a declared root
  that no declared pattern removes — an uncommitted source file changes what a
  compiler reads, and so does a file the ignore rules name, so both change the
  digest — and over content rather than modification times. The ignore rules
  are deliberately not consulted: they describe what a project declines to
  version, not what a checker reads, and material a checker really does read is
  routinely ignored, so letting them drop a path would leave the digest still
  while the checker's inputs moved. Generated material leaves a gate's inputs by
  being declared out, and naming a directory skips it whole. When a prior run of that gate recorded the same digest,
  the evaluation carries that run's verdict — passed or failed — instead of
  executing the checker. The reuse stays visible: the record names the run it
  was taken from, and `jit gate status --json` reports `inputs_digest` and an
  `origin` distinguishing `executed` from `reused`, so a container whose issues
  all report a passed gate is not read as that many independent verifications.
  Declaring inputs is what opts a gate in: a gate that declares none executes
  its checker on every evaluation, which is the right answer for a checker
  scoped to one issue or one that consults the clock, the network, or machine
  state, and `jit gate evaluate --force` re-executes a gate that would otherwise
  reuse.
- **A profile package declares the repository roots its live assets are drawn
  from.** Some of a package's assets are drawn from repository files the same
  repository also consumes at their working paths, and which directories those
  are was stated nowhere: the root set existed only implicitly, in the shape of
  the declared asset paths, so anything needing it — a step deciding which
  directories to watch, a check asking whether every live consumer was packaged
  — had to infer it or restate it. A manifest now carries one `[[live-source]]`
  entry per root, each with its own `exclude` list. Exclusions are shell-style
  patterns describing categories of repository material the package
  deliberately does not carry, because a literal list of those files would
  itself be a hand-maintained inventory: `*` stays inside one path segment and
  `**` spans segments, both matched against the repository-relative path. A
  root and an exclusion pattern are constrained types rather
  than free strings — a root that is not a relative path of ordinary segments,
  and a pattern that does not compile, are rejected where the manifest is
  parsed rather than by whichever consumer matches first — and a manifest
  declaring two roots that overlap is rejected as an invalid package, since a
  path beneath both would have two exclusion lists and no rule for choosing
  between them. The declaration is additive: a manifest carrying none parses
  unchanged and no existing key changes meaning. `jit profile show --json`
  reports it beside the assets it bounds, and the shipped `jit-dogfood` package
  declares the three roots its live assets come from.
- **A repository records where an applied profile package came from.** The
  record beside an applied profile carries the location of the package directory
  as a path relative to the worktree root. That location lives in the record and
  nowhere else: it is repository-local state versioned with the repository that
  applied the profile, and a configuration key carrying it as well would be a
  second carrier of one fact plus a precedence question between them. The
  recorded location is the one the package's own reader anchored its walk at,
  never a path supplied beside the bytes, so a package read through a relative or
  link-traversing argument records the directory those resolve to, and the record
  addresses the bytes it describes.
  Where the location may point is part of the contract: a package whose bytes
  resolve outside the worktree, including under the tracker's data root, is
  refused by name rather than recorded, because a location outside the worktree
  makes a repository's derived-state repair depend on machine state. A stored
  record whose location is absent or malformed fails the read rather than being
  ignored, so a repository never resolves a package its record does not actually
  name.

- **A profile package is read from a directory on disk.** The package model
  validates untrusted external data — bounded file count and total size,
  rejected absolute, traversal, platform-prefix, control-character and
  backslash paths, a declared source that is absent, a package file no
  declaration claims, and a content address over the manifest and every path.
  `ProfilePackage` owns the bytes it validates, and
  `ProfilePackage::from_directory` walks a directory into one path-to-bytes map,
  so packages built from identical content agree in manifest, package hash, and
  every per-target digest. No defence is weakened for directory packages. The
  reader rejects a directory entry that is neither a regular file nor a
  subdirectory, and an entry resolving outside the package root — which is how a
  symbolic link out of the tree is caught before its content is used. Both are decided against
  the entry the walk opened rather than against its name, and every entry is opened
  without following links, and without blocking, against the directory handle
  that listed it: a name relinked out of the tree while the package is being
  read fails to open instead of substituting content from outside it, and a name
  replaced by a pipe or a device is refused by name instead of holding the read
  open waiting for something to write to it. A package directory that is
  absent or unreadable reports that filesystem failure instead of an invalid
  package, so a location that does not exist is not diagnosed as bad content.
  Where a package directory comes from is decided elsewhere; this is the reader.

- **A stale shipped-policy region in the adopter configuration documents fails
  the mechanical documentation checks.** Generating those regions made them
  right once; nothing kept them right, so a change to the shipped
  development-area classification left them correct-looking until someone
  reran the generator. `scripts/docs-check-shipped-policy.sh` joins the
  `docs-mechanical` fan-out as M7 and reruns
  `scripts/generate-shipped-policy-regions.sh` against a scratch clone of the
  repository under check, carrying that repository's uncommitted tracked
  changes so the comparison is against the working tree while every write lands
  in a temporary directory. Drift is the difference between the fixture's tree
  object before and after that run, so the check states no classification
  value, no region marker, and no target path: the generator owns all three.
  `scripts/docs-check-selftest.sh` covers a fresh tree and a seeded stale
  region. The
  `docs-mechanical` gate description no longer lists its members, so adding one
  cannot make it stale.

- **The normal validation suites are callable from another workflow.** `ci.yml`
  accepts `workflow_call` beside its branch-push and pull-request triggers, so
  a branch build, a pull request, and any workflow of this repository that
  calls it run one maintained definition of the repository-validation, Rust,
  exact-MSRV, MCP, and web jobs on their own commit. The call takes no input
  and no secret, so a caller cannot aim the suites at a different commit.
  `.github/workflow-contract.yml` gains a `callers.require_needs` declaration
  for that promise: a reusable workflow names the jobs a caller inherits, and
  the verifier then requires each of them to exist and to carry no `if:`
  condition, and requires every job a caller runs of its own to reach the call
  through `needs`. A caller job that calls another workflow of this repository
  is exempt, so sibling calls run beside each other. Case vectors under
  `test-vectors/workflow-contract/` cover an unrouted caller job, an absent
  and a conditional promised job, a caller obligation on a workflow nothing can
  call, and the conforming shape.

- **CI validates the repository's own tracked data.** A `repo-validate` job in
  `ci.yml` runs `jit validate` with no issue id — every rule over the whole
  repository plus the repository-integrity checks — against a full-depth
  checkout, since integrity resolves document references pinned to a commit
  that a shallow checkout cannot look up. It is one of the jobs a caller of
  `ci.yml` inherits.

- **`jit-server` shuts down gracefully instead of dying mid-connection.** The
  server awaited `axum::serve` bare, with no signal handling at all, so a
  `SIGTERM` from `jit serve --stop` (or Ctrl+C under `jit serve --fg`) killed
  the process wherever it happened to be: in-flight responses were truncated
  and the exit was a signal death rather than an exit status. On a registered
  Ctrl+C or `SIGTERM` the server now cancels one process-wide
  `CancellationToken` — cloned into the application state and from there into
  every live SSE stream, so `/api/events/stream` subscribers reach EOF instead
  of being held open behind their 15-second keepalive — and then establishes
  its sole five-second drain boundary. It calls `graceful_shutdown(None)` on
  the `axum-server` handle that owns the listener and connections to stop
  acceptance and allow an indefinite cooperative drain, so the port is
  released at once and ordinary connections get the full boundary to finish.
  At that boundary JIT samples the connection count and calls `shutdown()` to
  force-close only survivors. The process exits `0`, and its log records the
  signal, the deadline, the connection count at the signal and at expiry, how
  long the drain actually ran before it ended, the forced-close path, and clean
  completion.

- **The declared MSRV is checked against current stable and proven by a test
  run.** `scripts/rust-version-policy.py` derives the declaration from
  `cargo metadata` and the current stable release from the Rust release channel
  manifest, both at run time, and reports whether current stable is at most one
  minor release ahead of the declaration. Its verdict is one JSON object naming
  both versions, the distance between them, and the window that distance was
  judged against; an underivable version exits `2`, keeping an unmade
  comparison separate from a stale declaration. CI's `msrv` job takes the
  declared version from that command instead of grepping `Cargo.toml`, runs the
  window check, and then both builds and tests the workspace `--locked` on the
  declared compiler, so `axum-server`, `tokio-util`, and the rest of the
  committed lockfile are exercised there rather than only compiled.
  `docs/reference/release-policy.md` is the policy's canonical home and states
  the refresh procedure.

### Fixed

- **Archive destinations no longer repeat the configured development root.**
  Artifact paths beneath a document or container destination are now relative
  to `documentation.development_root`, so `dev/active/plan.md` archives as
  `<container>/active/plan.md` instead of `<container>/dev/active/plan.md`.
  Planning, execution, relinking, recovery, and reruns share that canonical
  derivation, and the repository's existing archive trees and live links have
  been migrated to it.

- **An archive preview's `moving-path-citation` warnings no longer fire on
  citations that already name the archived destination.** The scan matched a
  moving artifact's source path as a plain substring. Before the canonical
  archive-relative layout above, a destination repeated the entire source path
  after its root, so a citation corrected to that pre-cutover destination still
  contained the source path and still warned — with the column shifted by the
  length of the inserted destination root.
  Repointing a citation therefore never emptied the report, and the warning set
  could not be used as a work list. The scan now reports an occurrence only
  where the surrounding text names the moving path whole: text naming a longer
  path that ends with it warns for neither, whether it is the published
  destination, an unrelated root above it, a directory whose name ends with its
  first segment, or a longer file name past its end. A citation of the source
  path alone is still reported, at the column where that path begins.

- **`jit doc check-links` honours a document reference's commit pin.** It read
  the working tree for every reference, so a reference pinned to a commit whose
  file had since been deleted was reported `missing_document` — permanently, for
  every historical reference the archival workflow pins by design — while `jit
  validate` resolved the same reference at its pin and passed. Both commands now
  apply one rule (`crate::document::resolve_document_reference` over
  boundary-captured evidence): a pinned reference resolves at its commit and an
  unpinned one in the working tree with a `HEAD` fallback. A pinned reference is
  read at its commit throughout — file, assets, and internal links — so its
  neighbourhood resolves as it stood at the pin. A pin is a claim about Git
  history, so without Git a pinned reference is reported unresolved carrying the
  boundary's reason, the answer `jit validate` already gave; unpinned references
  still resolve from the working tree, keeping link checking usable without Git.

- **Graph-template application no longer leaves a node `ready` while a
  dependency it just wired blocks it.** `jit apply` added the edge, then read
  the dependent's dependency set back from the already-mutated map. When the new
  edge introduced no transitive redundancy the set compared equal to itself and
  a guard skipped the rest of the loop body — which held both the `ready →
  backlog` demotion and the `dependency-add` event. Applying the planning
  bracket therefore left the breakdown node and the container `ready` with unmet
  dependencies, and recorded neither event. The guard now exempts the dependent
  whose set grew, and both paths that mutate a dependency edge — `jit dep
  add`/`rm` and template application — derive readiness through the single
  domain helper `Issue::derive_readiness_correction`. `jit validate` reports any
  issue stored as `ready` while carrying unmet dependencies, and `jit validate
  --fix` demotes it to the state its dependencies imply.

### Removed

- **The legacy gate-verb aliases `pass`, `pass-all`, `check`, and `check-all`
  are gone.** Issue 949cd9d0 renamed these verbs to `evaluate`, `evaluate-all`,
  `status`, and `status-all` and kept the old spellings as silent aliases so no
  caller broke at rename time. This change drops the four aliases: invoking
  any of them now fails with clap's standard unrecognized-subcommand error,
  the same as any other unknown command. The short alias `eval` (for
  `evaluate`) is unaffected and continues to work. `jit gate --help` and the
  schema's `gate` subcommand listing now show only the canonical verbs
  (`evaluate`/`eval`, `evaluate-all`, `fail`, `status`, `status-all`, plus the
  configuration and inspection verbs), and in-repo docs and tests were swept
  to the canonical spellings.

- **The split API/web images, the CLI image, and every container registry
  publication are gone.** Docker support was four overlapping topologies —
  `docker/Dockerfile.api`, `docker/Dockerfile.web`, `docker/Dockerfile.cli` with
  their `docker/entrypoint.sh` and `docker/nginx.conf`, plus the root all-in-one
  image — wired together by a Compose file that ran three services against a
  detached `jit-data` volume at `/data`, and published by a workflow that pushed
  six image names to `ghcr.io`. Docker support is now the root `Dockerfile`
  alone: one `jit-server` process serving the API and the built web UI on port
  3000 against a whole repository bind-mounted at `/repo`, so `.jit/` and the
  documents linked from it keep their repository context. `docker-compose.yml`
  declares that one service, mounting `JIT_REPO` and running as
  `JIT_UID`:`JIT_GID`, which a deployment derives from the served repository's
  owner; unset, the service runs as the image's fixed `10001:10001`. The
  container workflow builds and runtime-smokes that image on pull requests and
  `main`, and pushes to no registry: the image is something an operator builds
  from the repository. `scripts/test-podman.sh` drives the same contract suite
  under Podman, and `scripts/validate-setup.sh`, `INSTALL.md`, and
  `docs/how-to/deployment.md` describe the repository-mount contract.

### Changed

- **Regenerating a generated artifact of this checkout has one invocation
  form.** Six of the eight generators were `#[ignore]`d tests beside their
  asserting sibling, reached by naming the ignored test on a cargo command line
  — one of them by setting an environment variable while running the assertion
  itself — while the other two were scripts, so a contributor who learned one
  shape could not regenerate anything using the other. Every generator is now a
  script under `scripts/`, named after its artifact and taking no arguments,
  printing one `updated: <path>` line per file it rewrote and then one `OK: …`
  summary, and exiting 0 when the artifact holds its rendered values, 1 when the
  render or the publication failed, and 2 when the run was refused or the
  environment could not support it.
  `crates/jit/src/generated_artifacts.rs` declares the set — each artifact's
  target, its entry point, and where its render lives — and the drift assertions
  cite the same declarations, so an assertion and the repair it names read one
  render. A generator whose values come from library code renders through the
  single `regenerate` example rather than one example apiece, and one whose
  values can only be read out of an already-installed binary keeps its render in
  its own script. `dev/index.md` states the convention.

- **A repository decides for itself whether validation needs a profile
  package.** Whole-repository validation and derived-state repair each took a
  profile package by an identity compiled into the binary, before opening their
  mutation session and propagating any failure, so every repository paid for one
  whether or not it had ever applied a profile. Both paths now read the
  repository's own applied-profile records under `.jit/profiles/` and resolve a
  package only for the profiles those records name. A repository that has
  applied nothing reads no path plain validation does not. A record whose
  package resolves has its profile-owned targets checked and repaired exactly as
  before. A record whose package cannot be obtained fails, naming the record and
  the profile, rather than repairing the subset it can still account for —
  silently narrowing what `--fix` restores would breach
  `@/invariant/derived-state-coherence` where nobody would see it.

- **The shipped development-area classification reaches the adopter
  configuration documents by generation.** `docs/reference/configuration.md`
  restated all three area lists by hand and `docs/reference/example-config.toml`
  carried a reduced illustrative subset, with nothing binding either to the
  constant `jit init` renders, so both drifted silently. Each file now carries
  one marked region — `<!-- jit:shipped-documentation-policy:begin/end -->` in
  the reference, `# jit:shipped-documentation-policy:begin/end` in the example
  — that `scripts/generate-shipped-policy-regions.sh` fills with the
  `[documentation]` table a throwaway `jit init` writes into a temporary
  directory, the one route to the shipped values that reads no policy belonging
  to this repository. The example configuration therefore ships the
  classification a fresh repository actually receives. Because that table comes
  from the installed binary rather than from the working tree, the generator
  establishes currency positively before writing — the binary must report a
  build commit, that commit must resolve in the repository being written to,
  and the binary must report itself current there — and refuses on anything
  less, since an unknown build commit, an unresolvable head, and an unrelated
  repository all produce the same silence as a current binary.
  `scripts/generate-shipped-policy-regions-selftest.sh` holds the regression
  evidence for idempotence, for byte preservation outside the markers, and for
  each refusal arm.

- **Installation, deployment, MCP, and release facts each have one documented
  home.** The installation guide carried the container deployment, the web
  bundle build, and the MCP install beside the native archive, while the
  README, the deployment guide, and the package READMEs restated pieces of
  each, so one command lived in several places and drifted independently.
  `INSTALL.md` now owns the published archive — download, checksum
  verification, and the version, target and profile `jit version --json`
  reports — plus the contributor build from a source checkout.
  `docs/how-to/deployment.md` owns the container deployment including building
  the image, and the web bundle `jit-server` embeds. A new
  `docs/how-to/mcp-integration.md` owns installing the released MCP tarball and
  starting it from a client, and the CLI reference's MCP section points there
  for setup instead of naming the package README authoritative.
  `docs/reference/release-policy.md` owns the product-version procedure and the
  release's complete published output — one GitHub release per version tag, no
  package registry, no container registry, no separate web bundle — and cites
  the compatibility record the version contract reads rather than absorbing it.
  Every command, artifact name, mount path, health URL and prerequisite on
  those pages names the workflow, manifest, or image definition that keeps it
  true. The entry points that carried copies route to those homes instead:
  `README.md` sent MCP adopters to the component directory and restated the
  native install walkthrough, and `mcp-server/README.md` carried its own
  install and client-configuration walkthrough; each now links the guide that
  owns the workflow, while README keeps the product overview and the preferred
  quickstart and the package README keeps its component architecture.

- **The documentation gate fails when a canonical home rots.**
  `docs-mechanical` gains a fourth check, `scripts/docs-check-canonical.sh`
  (M6), over the canonical-home manifest `scripts/docs-canonical-homes.toml`.
  Each entry binds one adopter-facing fact to the page that owns it, the
  literal an adopter acts on, and the automation carrying that literal: the
  check reports MISSING when the home, its anchor, its statement, or its
  navigation link is gone, STALE when a source stops carrying what the page
  states, and DUPLICATE when a second scanned page states the same literal. Its
  scan covers the adopter documentation root together with the entry-point and
  component pages a reader arrives through — `README.md`,
  `mcp-server/README.md`, `contrib/README.md`, and `AGENTS.md` — since a copied
  adopter workflow otherwise hides exactly there. Where a contributor file
  legitimately spells the same literal for a different purpose, a
  `[[fact.binding.exempt]]` entry states the path and the reason, and an
  exemption whose page stops stating that literal is itself reported. The
  orchestrator appends every declared home to the resolved footprint, so the
  link and citation checks reach a canonical page outside the adopter
  documentation root — the installation guide is one. Each defect class is
  seeded and reverted against an isolated fixture repository in
  `scripts/docs-check-selftest.sh`.

- **The release is published by one tag-triggered workflow.** `release.yml`
  packaged binaries on any `v*` tag: it ran no validation suite and no security
  audit, never started what it packaged, carried no license text, and rendered a
  hand-written body advertising an npm install it never performed. Two workflows
  replace it. `release-publish.yml` answers a version tag alone, under a
  non-cancelling concurrency group that keeps two release runs from overlapping.
  It calls `ci.yml` and `security-audit.yml` on the tagged commit, builds and
  smokes every artifact downstream of both, verifies that the tag is annotated
  and names the version the manifests declare, and then creates one GitHub
  release carrying the Linux x86_64 musl archive, the MCP tarball, a SHA-256
  file covering both, both license texts, and `docs/release-notes/v<version>.md`
  as its body — reaching no package registry and no container registry, and
  creating, moving and deleting no git ref. `release-artifacts.yml` owns the
  build: the web bundle is a predecessor of the native build, since
  `crates/server/build.rs` embeds whatever `web/dist` holds when `jit-server`
  compiles and substitutes an empty stub when it is absent, and the smoke job
  extracts the archive into a clean prefix, runs `jit version`, `jit init`, the
  profile quickstart and the `--json` and `--schema` surfaces, and reads the
  document the archived server actually serves. That workflow also runs on every
  pull request, so the artifact path is exercised continuously rather than first
  on the release tag, and it holds no publication step for a pull request to
  reach. `.github/workflow-contract.yml` gains two assertions for the shape:
  `jobs.<name>.uses` states the workflow a caller job calls, so a `needs` edge
  keeps the content the call gave it, and `global.publication` names the one
  workflow that may create a GitHub release and forbids package- and
  container-registry publication everywhere, following local composite actions
  so a second publication path cannot hide one level down. Case vectors under
  `test-vectors/workflow-contract/` cover a replaced call, a release published
  from another workflow and from a composite it uses, a registry push inside and
  outside the publication workflow, a publication declaration naming an
  uncommitted file, and publication reaching past the validation and artifact
  calls. `scripts/release-version-contract.py --declared` prints the
  manifest-derived product version, which is how publication names the release
  note it renders without carrying a version literal of its own.

- **Debug info and incremental compilation are bounded by policy instead of
  Cargo's undocumented defaults.** Full debug sections dominated a
  representative test executable's size, and incremental state accumulated
  without bound across gate runs (baseline measured in
  `dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md`). The workspace manifest's
  `[profile.dev]` and `[profile.test]` now set `debug = "line-tables-only"`,
  keeping line-number backtraces without the full debugger payload, and both
  state `incremental = true` explicitly so ordinary interactive builds and
  test runs keep Cargo's incremental cache on purpose rather than by
  accident. `scripts/cargo-ci.sh` exports `CARGO_INCREMENTAL=0` for every
  step and now runs a dedicated
  `incremental-state` step afterward that fails the gate if any non-empty
  `incremental` directory remains under the target directory the run used:
  a gate run compiles once and exits, so incremental state has no later
  rebuild to amortize its cost against.

- **Documentation projections are declared generically and rendered by one
  command.** A single `[projection.<name>]` config registry (fields `kind`,
  `mode`, `target`, `style`, optional `region-begin`/`region-end`) drives every
  projection, rendered by `jit project render [--name <name>]`. This replaces the
  bespoke `[invariant_projection]` and `[rules_gates_projection]` config tables
  and the separate `jit invariant render` / `jit reference render` commands
  (removed). Any addressable item kind projects its `- **{id}** — {text}` rows
  through the generic `id-anchor` style with no dedicated code; the built-in
  `full` style renders the rich invariant and rule+gate registry views.

- **Profile manifests declare projections instead of singleton tables.** The
  `singleton-table` contribution (with its `invariant-projection` /
  `rules-gates-projection` targets) is replaced by a `projection` contribution
  (`kind = "projection"`, `name = "<projection-name>"`, and a complete `value`
  carrying `kind`/`mode`/`target`/`style`) that merges into the `[projection.*]`
  registry, so a profile's projection is byte-equal to the config an adopter
  reads.

- **The dependency audits block a pull request and every workflow that calls
  them.** `security-audit.yml` previously ran one job whose audits were advisory
  in practice: `cargo install cargo-audit` took no version, `npm audit
  --production` took no `--audit-level`, and the workflow answered to a weekly
  schedule, a lock-file push, and a manual dispatch but never to a pull request.
  It is now three jobs, one per committed lock — `cargo-audit`,
  `npm-audit-mcp-server`, and `npm-audit-web` — running `cargo audit -D
  warnings` after installing `cargo-audit 0.22.2` with `--locked`, and `npm
  audit --omit=dev --audit-level=info` against each `package-lock.json`. A
  failed toolchain or auditor install, an unreachable registry or advisory
  database, and any finding each end as a red job; the workflow carries no
  `continue-on-error`, shell fallback, ignore flag, or allowlist. `pull_request`
  — filtered by neither base branch nor changed path, because an advisory
  reaches a lock no commit touched — and `workflow_call` join the existing
  triggers. The workflow's `callers.require_needs` entry names all three
  boundaries, so every job a calling workflow runs of its own has to reach the
  call through `needs`, artifact production and publication alike. Two case
  vectors under `test-vectors/workflow-contract/` cover the conforming
  three-boundary shape and an artifact job wired beside the call while
  publication is wired behind it.

### Added

- **One static harness verifies every committed GitHub workflow.**
  `scripts/workflow-contract.sh` runs a pinned, checksum-verified `actionlint`
  and this repository's own structural verifier over the workflow tree, and the
  `workflow-contract` job in `ci.yml` runs both on every pull request. Each
  workflow declares what it guarantees in `.github/workflow-contract.yml` —
  required and forbidden triggers, `workflow_call` inputs and outputs,
  transitive `needs` edges, workflow- and job-level permissions, required jobs
  — instead of carrying a verifier of its own. Repository-wide rules reject any
  `continue-on-error`, any condition that survives a failed predecessor, any
  shell-level suppression of a non-zero exit, and any external `uses:` that is
  not an exact 40-character commit SHA with an upstream-version comment; the
  `uses:` scan follows local `./…` references into composite actions
  recursively. Twenty case vectors under `test-vectors/workflow-contract/` seed
  one defect class each, and `scripts/workflow-contract-selftest.sh` replays
  them plus an actionlint-only defect. `dev/workflow-contract.md` documents the
  declaration grammar and what a pin-update pull request has to establish
  before a maintainer merges it.

  Every external action in the CI, security, container, documentation, and
  release workflows moved to a resolved commit SHA in the same change, every
  workflow gained an explicit least-privilege token, and the `profile-adoption`
  test command was quoted — its `:: ` test-path filter made `ci.yml`
  unparseable to a strict YAML parser.

- **Automatic Rust build-footprint budget enforcement in the `cargo-ci` gate.**
  A committed checker, `scripts/rust-build-budget.sh`, derives the logical test
  topology and active-executable footprint from Cargo output rather than
  scanning stale `target/` artifacts: it counts integration-test targets from
  `cargo metadata` (fails above 12) and sums the unique active test-executable
  sizes from `cargo test --workspace --no-run --message-format=json` (fails
  above 2 GiB), and it asserts the debug-profile, gate-incremental, and
  dependency-feature (no remote JSON Schema resolution, one TLS backend)
  policies against the committed manifests and gate script. The budgets and
  their evidence are defined once in the checker and
  `dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md`. `scripts/cargo-ci.sh` runs it
  as a `budget` step after its test step, reusing warm Cargo artifacts (no
  second cold build), and folds a concise footprint summary into the persisted
  gate summary; over-budget or policy-drift runs fail with a diagnostic naming
  the observed value, the limit, and the corrective area. Each failure mode has
  an injectable-input regression fixture in
  `crates/jit/tests/scratch_build/rust_build_budget_checker_tests.rs` that runs
  without compilation.

- **Help cross-references from mutation/inspection commands to the reporting
  commands that answer "what happened".** `jit issue show --help` now names
  `jit issue status` (compact one-line view), `jit gate status-all`/`jit gate
  status <id> <gate>` (per-issue gate readiness/history), and summarizes the
  JSON response's top-level fields (`dependencies`, `unmet_dependencies`,
  `gates`, `documents`, etc.) so a caller doesn't have to run `--json` and
  inspect the shape to discover them. `jit issue create`/`update` and `jit doc
  add`/`remove` `--help` now point at `jit events query --issue-id`/`jit
  events tail` for verifying a recorded change. Gate failure output (`jit gate
  evaluate`'s error message and JSON suggestions, and the gate-blocked
  transition error's remediation) now also names `jit gate status <id> <gate>
  --all`, the run-history view, alongside the existing single-run and
  readiness commands. Top-level `jit --help`/`-h` now names `jit --schema` for
  JSON response shapes and exit code documentation.

- **Canonical hierarchy resolution in the core, shared by the CLI and web UI.**
  Parent, children, cluster, and rank per node are now resolved once in the core
  library (`jit::graph::hierarchy`) treating the **dependency DAG as
  authoritative and membership labels as advisory** — the model previously lived
  only in the web UI's TypeScript, forcing external tools to re-port it. A
  container is any type below the configured leaf level; a node's parent is the
  nearest dominating container, its cluster is the strategic root, and its rank is
  the longest dependency-path depth. New surfaces:
  - **`jit graph tree [<root-id>] --json`** emits the resolved parent/children/
    cluster/rank per node (`{count, root, nodes}` envelope); a root id scopes the
    view to that node's dependency closure.
  - **`jit graph export --format json --full`** nodes gain the same four
    additive resolution fields as `graph tree` (`parent`, `children`, `cluster`,
    `rank`). The default summary shape is byte-for-byte unchanged.
  - **`GET /graph`** on the web server carries each node's resolved `parent`,
    `children`, `cluster`, `rank`, and `type` value, computed by the core
    resolver over the repository's configured type levels.
  - **`jit --schema`** publishes the `graph tree` response shape
    (`GraphTreeResponse`) alongside the other command output schemas.
  - **`jit query divergence [--json]`** reports membership labels the DAG does not
    back (an issue labeled `epic:foo` that the `foo` epic does not depend on).
    `jit validate` surfaces the same as an advisory `divergence_count` that never
    changes its exit status.

  The web UI reads those served fields; the core resolver is the sole
  implementation, pinned by the fixture `test-vectors/hierarchy_resolution.json`.
  See [Hierarchy Resolution](docs/concepts/hierarchy-resolution.md).

- **Full-record bulk graph export (`jit graph export --format json --full`) and
  issue lifecycle timestamps.** The JSON graph export gains a `--full` flag that
  emits the complete issue record for each node — every field of the on-disk
  `issues/<id>.json` file, including `assignee`, `labels`, `gates_status` (each
  gate's key/status/`updated_by`/`updated_at`), `dependencies`, `description`,
  `created_at`/`updated_at`, and the new lifecycle timestamps — alongside the
  same `edges` list. This lets a bulk consumer read every node's full record in
  one call instead of globbing the issue files and streaming the event log.
  Without `--full` the output is byte-identical to the previous summary shape
  (`id`, `short_id`, `title`, `state`, `priority`, `labels` + edges); `--full`
  applies only to `--format json` (combining it with `dot`/`mermaid` is a usage
  error, exit 2).

  The issue record now stores three lifecycle timestamps written **once**, at
  the transition: `first_ready_at` (first time the issue enters `ready`,
  including the dependency-free auto-promotion at creation), `claimed_at` (first
  claim/assignment), and `done_at` (first time it reaches `done` — re-opening and
  re-completing does not overwrite it). All three are optional and omitted from
  JSON when unset. They are carried on the stored issue record and surface in the
  full-fidelity single-issue view `jit issue show --json` and in the `--full`
  graph export; the compact `issue status` projection stays lean and omits them.

  These fields are additive and optional, so they do **not** bump the repository
  `schema_version` (still `2`): the issue record does not use serde
  `deny_unknown_fields`, so an older binary ignores the unknown keys and a newer
  binary defaults them when reading an older file. Documented in
  [cli-commands.md § `jit graph export`](docs/reference/cli-commands.md#jit-graph-export)
  and [storage-format.md § lifecycle timestamps](docs/reference/storage-format.md#lifecycle-timestamps).

  **Migration for existing repositories:** run `jit migrate lifecycle-timestamps`
  once to backfill the timestamps for pre-existing issues from `.jit/events.jsonl`
  (first Ready transition, first claim, first Done transition). It is idempotent
  (a second run writes nothing) and fills only still-absent fields. Issues whose
  event log carries no relevant transition stay unset. `--json` reports
  `{issues_scanned, issues_updated}`.

- **Structured gate findings in machine output.** An automated checker can
  append a machine-readable block to its stdout, fenced by the line-exact markers
  `<<<JIT-FINDINGS-JSON` / `JIT-FINDINGS-JSON>>>`, carrying
  `{verdict, summary, findings:[{id, severity, summary, file?, line?}]}`. jit
  parses it once at gate-run record time and surfaces the parsed structure as a
  `findings` object on the run across the gate views (`gate status` latest-run
  and `--all` history, `gate status-all`, and the gate-blocked transition error
  envelope's `checker_result`). Raw stdout is kept alongside it, and the
  structure is retained even in the lean `status-all` projection that drops raw
  stdout for passing runs. A new findings view, `jit gate status <id> <gate>
  --findings`, prints only the verdict and findings — one greppable finding per
  line as text, or `{key, run_id, has_findings, verdict, summary, findings}`
  with `--json`. The contract is opt-in and degrades gracefully: a checker that
  emits no block, or a malformed block, yields no `findings` field and no error,
  leaving existing plain-text behaviour unchanged. Documented in
  [custom-gates.md](docs/how-to/custom-gates.md#structured-findings-machine-readable-output);
  the bundled `contrib/gates/ai-review.sh` is the first conforming checker.

- **`jit issue children <id>` — a container's direct children at a glance.**
  Lists the container's immediate dependencies (depth 1), each rendered exactly
  like `issue status` (one greppable line, ascending short-id order), or as the
  `{container: {short_id, title, state}, count, issues: [...]}` envelope with
  `--json` (`issues` is the same compact status projection). Containment follows
  the dependency DAG — a container's children are the issues it directly
  depends on; membership labels are advisory and not consulted. A non-container
  leaf simply lists nothing; for a deep rollup use `jit graph deps <id>
  --depth`. Replaces the per-child `show`/`status` loop agents ran to see where
  each child stands. A dependency edge pointing at a missing issue is surfaced
  in an optional `dangling` array (text: a `dangling:` line) rather than
  silently dropped, following the `issue show` `dangling_dependency_ids`
  precedent; a real storage error still propagates.

- **`jit issue progress <id>` — counts by state and a done/total rollup over a
  container's direct children.** Text prints the container line then `by state:
  backlog=… …` and `done <done>/<total> (<percent>%)  open …  rejected …`;
  `--json` emits `{container, count, by_state:[{state,count}], total, done,
  rejected, open, percent}`. `by_state` lists every lifecycle state (zero-count
  states included). Terminal-state semantics: `done` and `rejected` are counted
  distinctly (a rejected child is terminal but not delivered), `open` is every
  non-terminal child (`total − done − rejected`), and `done/total`/`percent`
  measure delivery. Totals cover resolvable children only; a broken dependency
  edge is surfaced in `dangling` (as for `issue children`). Membership follows
  the dependency DAG.

- **`jit query count --by state [--label ns:v ...]` — the same state rollup over
  a label bucket.** Aggregates every issue matching all `--label` patterns
  (ANDed; none given aggregates the whole repository) into the same
  `{count, by_state, total, done, rejected, open, percent}` shape as
  `issue progress`, minus the `container` header. This is the advisory-grouping
  counterpart to `issue progress`: DAG containment for the former, shared labels
  for the latter. `--by` is typed (`state` today); an unknown value is a usage
  error (exit 2). State counts enumerate the domain `State` enum, so the shape
  stays complete and stable.

- **`jit issue status <id>...` — the compact "where does this issue stand"
  view.** Prints state, per-gate status, and still-unmet dependencies as one
  greppable line per issue (`<short_id> [<state>] gates: <key>=<status>,...
  unmet: <short_id>,... title: <title>`; empty sections read `none`), or as one
  small object per issue with `--json`
  (`{short_id, state, gates:[{key,status}], unmet_dependencies:[short_id,...],
  title}`). It accepts multiple ids in argument order; two or more with `--json`
  use the `{"count": N, "issues": [...]}` list envelope. This replaces the
  hand-rolled `jq`/`python` projections agents previously reconstructed from
  full issue JSON. The unmet-dependency filter follows readiness semantics — a
  dependency is met exactly when it is terminal (`done`/`rejected`), the same
  test `jit query ready` applies.

- **`issue show --json` now exposes `unmet_dependencies`.** The full record
  gains an `unmet_dependencies` array — the subset of `dependencies` that are
  not yet met (state not terminal), each as `{id, short_id, title, state}` —
  computed by the same readiness-consistent predicate. Additive: existing fields
  are unchanged, and the array is always present (empty `[]` when nothing is
  blocking), so callers no longer recompute the filter client-side.

- **`jit config get` now covers the whole configuration surface.** The
  dotted-key accessor previously recognized only a hand-mapped subset
  (`worktree.*`, `coordination.*`, `global_operations.*`, `locks.*`,
  `events.*`); it now walks the full `config.toml` schema generically,
  including `type_hierarchy` (e.g. `type_hierarchy.strategic_types`,
  `type_hierarchy.types.epic`), `namespaces` (e.g.
  `namespaces.type.unique`), `item_kinds`, `documentation`, `validation`,
  `project`, and `version`. An intermediate key returns the whole subtree
  (`jit config get documentation`) rather than erroring; an unknown
  top-level key fails exit 2 naming the valid sections, and an unknown
  nested key fails exit 2 naming the missing segment. The five
  system/user/repo-layered sections keep resolving exactly as before;
  every other section reads the repo's `config.toml` only, with no
  built-in defaults layered in.

- **Wrong-verb hints for observed wrong-guess spellings.** `jit dep
  remove`/`delete`, `jit issue rm`/`remove`/`complete`/`edit`, `jit gate
  rm`/`delete`, `jit doc rm`/`delete`, and `jit label add`/`rm`/`remove` now
  fail fast (exit 2) with a message naming that group's canonical command
  (`jit dep rm`, `jit issue delete`, `jit issue update --state
  done`/`--label`/`--remove-label`, `jit gate remove`, `jit doc remove`)
  instead of clap's generic "unrecognized subcommand" error. These are hints,
  not new aliases — the wrong verb still fails, and canonical spellings are
  unchanged. `jit label --help` now also clarifies that the `label` group
  manages the namespace registry, not an issue's labels.

- **Repeatable, AND-combined `--label` filter across the query family.**
  `jit issue list`, the top-level `jit list` alias, the bare `jit query` form,
  and `jit query all`/`available`/`blocked`/`strategic`/`closed` now accept
  `--label`/`-l` multiple times; an issue is returned only when it matches
  every pattern given (wildcard `namespace:*` patterns still supported per
  occurrence). This matches `jit issue search --label`'s existing repeatable
  AND semantics. A single `--label` occurrence behaves exactly as before.

- **`jit init --json` and `jit graph export --json`.** `jit init` accepts
  `--json`, reporting `{repository_root, data_dir, repository_id,
  hierarchy_template, created_paths, modified_paths, message}` —
  `repository_id` is the git worktree id (`null` outside a git repository),
  `created_paths` lists only the files this run created (empty on a
  re-init), and `modified_paths` covers the one file init can update in
  place: an existing `.gitattributes` without the jit merge-driver block
  gets the block appended and is reported there rather than in
  `created_paths`. An unknown `--hierarchy-template` name emits the standard
  `--json` error envelope (`INVALID_ARGUMENT`, exit `2`). `jit graph export`
  gains `--json`, sugar for `--format json` on stdout; combining it with an
  explicit `--format dot`/`--format mermaid` is a usage error (exit `2`), and
  it composes with `--full`.

### Changed

- **Uniform JSON list envelope across list- and query-family commands.** Every
  command that emits a collection now wraps it in `{"count": N, "<collection>":
  [...]}`, where `count` equals the length of the collection and the key is
  plural and collection-typed. A single parse path now works for every
  list/query command — agents no longer need bare-array or dual-shape
  fallbacks. Collection keys: `issues` (`issue list`, `list`, `query
  all`/`available`/`blocked`/`strategic`/`closed`, `issue search`, multi-id
  `issue show`), `results` (`search`), `gates` (`gate list`), `presets` (`gate
  preset list`), `events` (`events tail`/`query`), `documents` (`doc list`),
  `assets` (`doc assets list`), `leases` (`claim list`/`status`), `namespaces`
  (`label namespaces`), `values` (`label values`), `templates` (`config
  list-templates`), `items` (`item list`/`search`), `worktrees` (`worktree
  list`), `roots` (`graph roots`), `dependents` (`graph rdeps`, `rdeps`),
  `nodes` (`graph deps`), `results` (`gate status --all`/`--limit`),
  `gates` (`gate status-all`), and `findings` (`invariant check`). Inner
  record shapes are unchanged. Where a command already carried an aggregate
  count, `count` is the size of the named collection specifically: `graph deps`
  keeps `summary.total` (unique dependencies across the whole tree) while `count`
  is the number of top-level `nodes`; `gate status-all` keeps `total` / `passed`
  (readiness tallies) while `count` is the number of `gates` entries.
- **Unified gate field naming across JSON outputs.** Everywhere a gate appears
  in `--json` output it is now identified by `key` and its state by `status`,
  matching `issue show`'s existing `gates[].key` — the larger, pre-existing
  surface. See the Migration section below for the renamed fields.
- **Typed exit codes for prefix and batch-usage errors; JSON envelope for
  startup failures.** Argument-class failures that previously fell through to the
  generic exit `1` are now classified:
  - An **ambiguous id prefix** (matches multiple issues) and a **too-short id
    prefix** (fewer than 4 characters) are argument errors (exit `2`), carrying
    the distinguishing `code` `AMBIGUOUS_ID` / `INVALID_ID_PREFIX` under
    `--json`. Human messages are unchanged.
  - The **batch-mode usage guards** on `jit issue update --filter` (mutually
    exclusive id/filter, and the `--content-format` / `--type` / description-flag
    rejections) now exit `2`, matching clap's own usage errors, and emit a JSON
    envelope under `--json`.
  - The **misplaced query-filter guard** (`--state`/`--assignee`/`--priority`/
    `--label`/`--full`/`--json` given before a `jit query` subcommand, where they
    would be silently dropped) now exits `2` and emits a JSON envelope under
    `--json`, matching the other query-family usage guards.
  - `jit dep rm <from> <target>` now validates **both** id arguments identically:
    a too-short or ambiguous prefix in either position is the same argument error
    (exit `2`), where previously a short `<target>` was silently reported as "not
    found" (exit `0`) while a short `<from>` exited `1`.
  - `jit dep add <from> <target>...` now emits the refined `INVALID_ID_PREFIX` /
    `AMBIGUOUS_ID` code (exit `2`) under `--json` for a too-short or ambiguous
    prefix in either the `<from>` or any `<target>` position, instead of the
    generic `DEPENDENCY_ERROR` (exit `1`). The non-`--json` exit code was already
    `2`; this aligns the `--json` code with it.
  - **Startup failures under `--json`** (repository not found, repository format
    too new) now emit a structured error object on stdout (`code`
    `REPOSITORY_NOT_FOUND` / `REPOSITORY_FORMAT_TOO_NEW`) while keeping their
    exit codes (`3` / `10`) and the human line on stderr. Previously `--json`
    produced an empty stdout for these.

### Fixed

- **`jit dep add` with multiple targets is now atomic.** Previously, a variadic
  add (`jit dep add <from> <to1> <to2> ...`) applied edges one at a time, so an
  edge that failed validation (e.g. a redundant edge under the default
  `--reduce`-less policy) left any earlier, already-applied edges persisted —
  the exit code no longer meant "nothing changed." Every requested edge is now
  validated against the would-be-final graph (every edge of the call applied at
  once, so a violation that only emerges from the COMBINATION of two edges in
  the same call is also caught) before anything is written; if any edge fails,
  none of them are added and no event is logged for any of them. The error now
  names every rejected edge, not only the first, and under `--json` carries a
  `details.rejected` array of `{from, to, code, message}` per rejected edge. A
  batch mixing an id-resolution failure with a graph-validation failure exits
  with the resolution failure's code (resolution runs before graph
  validation).
- **Doubled "Error: Error:" prefix on `ActionableError` paths.** Any command
  surfacing an `ActionableError`-derived failure (e.g. an already-claimed
  lease, a missing acting identity, the claims-require-git failure) now prints
  exactly one `Error:` prefix. `ActionableError::to_error_message()` no longer
  embeds its own prefix; the top-level CLI printer is the sole place that adds
  it.
- **`jit issue claim` on an issue already assigned to a different assignee**
  now names the current assignee and states that re-claiming as that same
  assignee succeeds (and promotes it to `in_progress`), instead of the bare
  "Issue is already assigned".
- **`jit claim acquire`/`jit claim release` outside a git repository** now
  distinguishes two causes instead of always suggesting `git init`: no git
  repository at all (still hints `git init`) vs. a git repository with no
  commits yet, so `HEAD` doesn't resolve to a branch (hints making an initial
  commit instead). Same typed error and exit code (`10`) for both; only the
  message differs.
- **`jit issue claim --help`** documents the idempotent same-assignee
  re-claim and the in_progress promotion.
- **Help text cross-references between assignment and lease commands.**
  `jit issue assign`/`claim`/`release`/`unassign` (assignee bookkeeping) and
  `jit claim acquire`/`release` (exclusive, time-boxed leases) share verbs but
  are different mechanisms; each command's `--help` now names its counterpart.
- **A downstream reader closing the pipe mid-write no longer panics.** Piping
  any command into something that exits early (`jit query all | head -1`,
  `jit --schema | head -c1`) used to surface Rust's raw panic banner (`thread
  'main' panicked ...: Broken pipe (os error 32)`, plus a backtrace hint) and
  exit `101`, because most of `main.rs`'s output goes through direct
  `println!`/`print!` calls that panic on a write error. `jit` now exits
  quietly with `141` (`128 + SIGPIPE`, the exit status a shell reports for a
  process a signal actually terminated), matching how everyday Unix pipelines
  compose.

### Migration

- **BREAKING — `jit dep add` with multiple targets no longer partially
  applies on failure.** A variadic add where one target fails validation used
  to leave every edge before the failure persisted; it now leaves the
  dependency set completely unchanged. Scripts that relied on the partial
  application (e.g. retrying only the failed target) must instead retry the
  whole batch. The `--json` success/error response no longer includes an
  `errors` array — a failure is now the command's `Err`/nonzero-exit path,
  carrying every rejected edge under `error.details.rejected` instead.
- **BREAKING — exit codes for prefix and batch-usage errors changed from `1` to
  `2`.** Scripts that branch on the exit code of an ambiguous/too-short id prefix,
  a `jit issue update --filter` usage guard, a misplaced pre-subcommand `jit
  query` filter, or `jit dep rm` with a bad id must
  treat `2` (invalid argument) as the failure code for these cases. A short
  `<target>` to `jit dep rm` that previously succeeded (exit `0`, reported under
  `not_found`) now fails with exit `2`; pass a ≥4-character prefix or the full id.
  `jit dep add` with a too-short/ambiguous prefix already exited `2`, but its
  `--json` `code` changes from `DEPENDENCY_ERROR` to `INVALID_ID_PREFIX` /
  `AMBIGUOUS_ID`. Consumers on `--json` can branch on the new `code` values
  (`AMBIGUOUS_ID`, `INVALID_ID_PREFIX`) instead of the exit code. No
  human-readable messages changed.
- **Additive — startup failures emit JSON on stdout under `--json`.** Callers of
  any command with `--json` in an uninitialized repository, or against a
  repository whose on-disk format is newer than the binary, now receive a parsable
  `{"error": {...}}` object on stdout (previously stdout was empty). Exit codes
  (`3` / `10`) and stderr are unchanged; consumers that only read the exit code
  are unaffected.

- **BREAKING — `jit issue show <id> <id> …` with `--json`.** Passing two or more
  ids previously emitted a bare JSON array (`[ {…}, {…} ]`). It now emits the
  list envelope `{"count": N, "issues": [ {…}, {…} ]}`. Consumers that indexed
  the top-level array must read `.issues` instead (e.g. `jq '.issues[]'` in
  place of `jq '.[]'`). Single-id `issue show --json` is unchanged: it still
  returns a bare issue object.
- **BREAKING — `jit graph deps --json` node collection renamed.** The node
  collection key changed from `tree` to `nodes` (`{"count": N, "nodes": [...]}`).
  Consumers must read `.nodes` instead of `.tree`; each node's inner shape
  (including nested `children`) is unchanged, and `summary` / `issue_id` /
  `depth` remain alongside.
- **Additive — `count` field added.** `gate preset list`, `doc assets list`, and
  `gate status-all` gained a top-level `count` alongside their existing
  collection. `doc assets list` continues to carry its `summary` object; its
  `count` mirrors `summary.total` (the length of `assets`). `gate status-all`
  keeps its `results` / `not_run` / `total` / `passed` keys; its `count` is the
  length of `gates`, not a readiness tally. Existing consumers that
  ignored unknown keys are unaffected.
- **Documentation — `gate status --all`/`--limit`.** The history view already
  emitted `{"count": N, "results": [...]}`; the envelope is now stated in the
  command help and the CLI reference (no shape change).
- **BREAKING — gate identification unified to `key` across all JSON output.**
  Every place a gate previously appeared under `gate_key` now appears under
  `key`; `gate status-all`'s collection also renamed `gate_statuses` to
  `gates`. Affected shapes:
  - `gate status-all` (`check-all`): `{"count": N, "gates": [{"key": ..., "status": ...}], ...}`
    (was `gate_statuses`, entries carried `gate_key`). The `results` entries
    (one per recorded automated run) also switch from `gate_key` to `key`.
  - `gate status`/`check`: the latest-run view, the `--all`/`--limit` history
    view's `results` entries, and the `--stdout`/`--stderr` flat view all carry
    `key` instead of `gate_key`.
  - `gate evaluate`/`pass` and `gate fail`: the success response's `gate_key`
    field is now `key`.
  - `gate evaluate-all`/`pass-all`: each entry in the `gates` array carries
    `key` instead of `gate_key` (the `gates` collection key itself was already
    correct).
  - `gate define` and `gate remove`: the response's `gate_key` field is now
    `key`.
  - A gate-blocked transition error's `error.details.blockers[]` entries
    (`type: "gate"`) carry `key` instead of `gate_key`.
  - A failed `gate evaluate`/`pass` error's `error.details` carries `key`
    instead of `gate_key`, and its nested `checker_result` is now the same lean
    run-summary shape used elsewhere (`key`/`status`/...) instead of the raw
    stored gate-run record (so it also drops `schema_version` and a duplicate
    `issue_id`).
  - The web UI's single gate-run endpoint (`GET
    /issues/:id/gate-runs/:run_id`) now returns the same run-summary shape
    (`key`, no `schema_version`/duplicate `issue_id`) instead of the raw stored
    record.
  Consumers must read `.key` wherever they previously read `.gate_key`, and
  `.gates` wherever they previously read `.gate_statuses` from `gate
  status-all`. The on-disk gate-run storage format
  (`.jit/gate-runs/**`) and `events.jsonl` gate-related event entries are
  unaffected — both keep `gate_key` as an internal/audit field; only the
  `--json` command output surface changed.

### Removed

- **The `jit::storage::lease` module.** It declared a second `Lease` type,
  superseded by `claim_coordinator::Lease` — which is what `jit::storage::Lease`
  re-exports and what every claim path uses. Nothing in the workspace reached
  the older one. The two were different designs rather than two names for one:
  the removed type carried an unserializable monotonic `Instant`, a wall-clock
  fallback for when that `Instant` was absent, and a `from_serde` reconstruction
  that approximated it across a reload, while the canonical type is plain data
  whose `is_expired` takes the instant as a parameter. Callers use
  `jit::storage::Lease` (`@/invariant/canonical-cutover`).

### Changed

- **`RepoWriteLock::acquire` reports an expired in-process wait by type.** A
  wait for another thread of the same process to release returned a plain
  string error, while the cross-process file-lock wait returned
  `jit::storage::lock::LockTimeout`. Both now return `LockTimeout`, so a caller
  distinguishing "I am still queued" from "the operation was refused" matches
  the type through `is_lock_timeout` rather than the message.

- **The shutdown drain takes its whole schedule from its caller.** The drain
  samples the live connection count until its deadline expires, and the wait
  between two counts was fixed inside the loop while the deadline arrived from
  outside it. Both now arrive together, and the serving process passes the same
  interval it has always enforced, so the deadline a stalled connection receives
  is unchanged. The case for a connection that finishes in the last sampling
  interval is what the seam is for: it used to give the drain four 100 ms
  intervals of deadline, sleep 3.5 of them and then complete the connection, so
  50 ms of host scheduling separated a reported drain from a reported forced
  close. Supplying a boundary the case triggers itself, and no periodic wake-up,
  orders its three events by observation — the drain reports the sample that
  found the connection live, the handle reports the retirement, and only then
  does the deadline expire — so a slow host delays each step instead of changing
  which branch the drain takes.

### Fixed

- **`jit serve` reports the start on what the server did.** The daemonizing
  start spawned the server, slept 300 milliseconds, and asked once whether the
  child had exited. A child that exited at 301 milliseconds was reported as a
  started server and left a PID file naming a process that was not running —
  the stale record that check exists to prevent — and the failure was likelier
  the slower the host, which is when an adopter can least afford to be told the
  wrong thing. The start now follows the child until it answers on its port or
  exits, whichever comes first, and reports that. An exit is a startup failure
  whenever it happens, a server that answers is reported at the moment it
  answers rather than after a fixed pause, and a child that does neither ends
  the start at a bound, is terminated, and is reported as unreachable, so no
  start can wait unboundedly and none leaves a survivor nothing is tracking. The
  bound tracks the repository's own configured storage-lock timeout plus room to
  serve past it, so raising that timeout raises what a start will follow rather
  than terminating a server still waiting for a lock it is entitled to wait for.
  The PID file is written only after the server has answered. What answering means,
  how often it is observed, and how long the start follows the child all arrive
  from the caller, so the cases decide on the outcome the start produced: one
  releases a blocked child through a FIFO on the very observation the start
  makes, so the child's exit strictly follows an observation of it running and
  no pause of any length could have concluded otherwise. Restoring the "it has
  not died yet" conclusion fails them.

- **The daemonizing `jit serve` releases the recovery lock its child needs.**
  The parent takes the bootstrap recovery lock at startup, and the foreground
  start already released it before waiting on its child. The daemonizing start
  did not, and the omission was invisible only because that start returned
  moments after spawning and the parent then exited; a start that follows its
  child holds the lock while the child waits for it, so the child times out and
  dies. Both starts now release the session at the same point, before either
  begins waiting. Two cases cover the pair end to end: the foreground one that
  already existed, and one asserting a reported start names a server answering
  on the port it reported.

- **The orphan-cleanup case reaches the orphan cleanup.** It blocked the PID
  write by placing a directory at the PID file path, which `start_server` reads
  before it spawns anything: the start failed at that first read, having spawned
  no child, and the case's assertions — that the error mentions PID persistence
  and that the blocker survives — both held without any of the cleanup running.
  The blocker now sits on the path the atomic write stages through, so the start
  spawns its child, succeeds at everything up to the write, and fails there; the
  case asserts that the child it spawned is gone and that no record was
  published.

## [1.0.0] - 2026-07-30

The first stable release. [The v1.0.0 release
note](docs/release-notes/v1.0.0.md) is the source the published release body is
rendered from.

### Added

- **One product version spans every supported capability.** The CLI, the
  server and its API, the built web UI, and the MCP server ship under a single
  version. [The product compatibility
  record](docs/reference/compatibility.md) enumerates that capability set,
  draws the boundary against the independent format version that governs
  repository data, and states the upgrade expectations: an upgrade replaces the
  installed artifacts together and leaves repository data in place.

- **The repository carries both texts of its declared `MIT OR Apache-2.0`
  license.** `LICENSE-MIT` and `LICENSE-APACHE` sit at the repository root and
  ship inside the published native archive. The workspace manifest declares the
  copyright holder once; the crate manifests inherit it, the npm manifests
  restate it, and the MIT copyright line is checked against that declaration
  rather than maintained by hand. The release-metadata check refuses a release
  whose license texts, changelog entry, compatibility-and-upgrade record, or
  release-note source is missing or names a version other than the declared
  product version.

### Removed

- **Publication paths outside the single GitHub release.** v1.0.0 publishes one
  GitHub release carrying the native archive, the MCP server tarball, their
  SHA-256 checksums, and both license texts. The MCP server is installed from
  that release tarball rather than a package-registry version; container images
  are no longer published to a registry, and no source distribution is produced
  beyond the source archives GitHub generates for the tag.
