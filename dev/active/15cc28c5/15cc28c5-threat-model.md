# Threat model — portable package exchange (`15cc28c5`)

Precheck threat model for the `security-review` gate. It covers the six areas
that gate names — path containment, symlink and junction races, recovery
journals, untrusted manifest content, dependency risk, and secrets — against the
pack/add surface this issue introduces, and fixes the design constraints the
implementation must satisfy.

The trust boundary is stated by the epic: an archive arrives from outside the
repository over a channel the adopter chose (`@/charter/D-8` epic decision D-02),
so `add` treats every byte of it as hostile input.

## 1. Path containment

An archive entry names its own destination path, so entry names are the primary
injection surface (`zip-slip`).

- An absolute entry path, a `..` component, a root-relative path, or a drive or
  UNC prefix is refused before anything is written.
- The destination must classify as worktree content through the repository
  layout, exactly as capture's destination does. Anything under the separate
  data root, or outside the repository, is refused.
- Containment is established by the open, not by comparing strings. Wave 10
  settled this: `crates/jit/src/profile/nofollow.rs` anchors a `CapDir` and opens
  beneath it, so an escaping resolution is refused by construction. Extraction
  reuses that module. A `starts_with` check on a joined path is not acceptable —
  it is the shape that failed `6a479c58` review round 1.

## 2. Symlink and junction races

- Tar carries entry types beyond regular files. `Symlink`, `Link`, `Char`,
  `Block`, `Fifo`, and `Socket` entries are refused outright; only `Regular` and
  `Directory` are admitted. A package is a manifest plus regular files, so
  nothing legitimate is lost.
- Modes are not taken from the archive. The manifest declares which assets are
  executable; a mode an entry carries that its declaration did not is refused
  rather than published. This keeps the executable contract on the manifest,
  where capture already put it.
- Time-of-check to time-of-use: extraction stages into a fresh private directory
  and publishes whole. No path is checked and then reopened by name.

## 3. Recovery journals and publication

- REQ-04 requires that a refused or interrupted add leave no partial package and
  never overwrite an occupied destination.
- Wave 10 deleted `publish_staged_directory_noreplace`; the surviving no-replace
  publisher is `publish_external_directory_noreplace`
  (`crates/jit/src/storage/external_publish.rs`), whose consumer is snapshot
  export. Add either uses it or rides the shared recoverable transaction as
  capture does. It must not introduce a third publication path
  (`@/invariant/canonical-cutover`, `@/invariant/convention-convergence`).
- The staged tree is validated as a package *before* publication, so a tree that
  fails validation is never visible at the destination.

## 4. Untrusted manifest content

- The manifest inside an archive is attacker-controlled. It is decoded by the
  existing strict package decoder with its `deny_unknown_fields` contract; no
  relaxed parse path is introduced for archives.
- The package bounds the model already imposes — `MAX_PROFILE_PACKAGE_FILES` and
  `MAX_PROFILE_PACKAGE_BYTES` — are enforced **during** extraction, against
  running counts, not after the bytes have landed. Enforcing after extraction
  turns a size bomb into a disk-exhaustion primitive.
- Entry count is bounded for the same reason, so an archive of many empty
  entries cannot exhaust inodes.

## 5. Dependency risk

- `tar = "0.4"` is already a non-dev dependency (`crates/jit/Cargo.toml:33`) and
  is already used to build snapshot archives (`commands/snapshot.rs:634`). No new
  crate is introduced.
- The archive is **uncompressed**. Adding a compression layer would introduce a
  decompression-bomb vector whose bound is not knowable before inflation, widen
  the build footprint against
  `@/invariant/bounded-rust-build-footprint`, and add a dependency to the
  `dependency-audit` surface. If compression is later wanted, it is a separate
  decision with its own threat model.
- REQ-05 forbids network access, registry lookup, and undeclared filesystem
  search, so no resolver or fetch path exists to attack.

## 6. Secrets

- Profile variables are non-secret by declaration, and the profile surface
  carries no secret-value channel (epic REQ-04, RISK-03).
- Pack reads the package directory only. It must not read resolved values, an
  applied record, or anything outside that directory, so an archive cannot
  become a channel that carries repository state off-site.
- Packing is over unresolved package bytes, matching how package identity is
  already computed.

## What the digest does and does not establish

REQ-01 has the archive carry the package identity digest and REQ-02 has add
recompute it from extracted content and refuse a mismatch. This is **integrity,
not authenticity**.

The digest travels inside the archive, so anyone who can rewrite the content can
rewrite the digest. It detects truncation, corruption in transit, and accidental
modification. It does not establish who produced the package, and it is not a
signature.

This is consistent with the epic's decision that the adopter obtains an archive
over a channel they already trust (D-02): the channel carries the authenticity,
the digest carries the integrity. Adopter documentation must not describe the
digest as proof of origin. Recorded here so the eventual documentation
(`b3d92595`) states the boundary correctly rather than overclaiming.

## Residual risks accepted

- A hostile archive whose manifest is internally valid can still describe a
  package an adopter did not want. Nothing here judges package *intent*; add
  places a package in the worktree, and applying it remains a separate,
  explicit operation with its own planning and rehearsal.
- An empty directory left by wave 10's whole-tree republication (no
  `DeleteDirectory` in `RepositoryAction`) can be carried into an archive. It is
  inert, and the published tree still decodes as a package.

## Verdict

The surface is safe to build under the constraints above. Every constraint in
sections 1–4 is a hard requirement on the implementation, not advice.
