# Research: profile publication and binary embedding choices

**Container:** `9b7b5f9c` — Add first-class reusable jit configuration profiles
**Scope:** two implementation choices for the embedded `jit-dogfood` MVP only
**Date:** 2026-07-14

This note is deliberately narrower than an implementation plan. It excludes local profile
packages, multiple-profile composition, variables, upgrade, and removal, as required by the
MVP boundary (`dev/active/9b7b5f9c-mvp-scope-brief.md:28-30`).

## Decision 1: durable staging journal with reversible publication

Choose a **write-ahead staging journal with rename-aside originals, a durable commit decision,
and mandatory recovery**, all under the existing repository-wide write lock. Do not use an
in-memory “copy backups, write targets, and roll back on `Err`” sequence as the transaction
contract.

This is new storage-layer machinery, not a claim that the current atomic writer already forms a
transaction. The existing writer stages one sibling file and renames it over one target
(`crates/jit/src/storage/atomic_write.rs:13-42`); ruleset publication repeats that operation for
multiple files without set-wide recovery (`crates/jit/src/storage/ruleset_store.rs:28-50`). The
artifact publisher contributes useful no-follow staging, identity verification, and no-replace
publication (`crates/jit/src/storage/artifact_mutation.rs:135-217`,
`crates/jit/src/storage/artifact_mutation.rs:219-297`), but its stage write only calls `flush`,
which is not a durability barrier (`crates/jit/src/storage/artifact_mutation.rs:167-200`).

### Required protocol

1. **Plan without mutation.** Read the embedded package and repository, compute every final byte
   string and executable-mode action, classify absent/identical/managed replacement/conflict,
   calculate hashes, and validate the complete virtual final repository. A conflict ends here.
   This is the validated-first behavior required by REQ-04
   (`dev/active/9b7b5f9c-mvp-scope-brief.md:18`). Identical targets remain byte-for-byte
   untouched; a wholly unchanged reapplication returns success without a journal or event.
2. **Lock and recheck.** Acquire the existing outer `RepoWriteLock`, then rebuild or revalidate
   the complete plan before staging. That lock is repository-local and git-independent, and its
   contract already covers read/validate/write/rollback sequences
   (`crates/jit/src/storage/repo_lock.rs:1-30`,
   `crates/jit/src/storage/mod.rs:87-106`). Any event-file mutation also takes the existing event
   lock after the repository lock, preserving the documented lock order
   (`crates/jit/src/storage/repo_lock.rs:26-30`).
3. **Prepare durable state.** Create a transaction directory under `.jit/tmp/` containing the
   final staged files and a versioned journal. The journal records transaction ID, deterministic
   plan hash, each target's expected original identity and relevant mode, final identity/mode,
   stage/backup names, action order, and phase. Set Unix executable bits on the staged inode
   before publication; on non-Unix platforms record that this mode action is not applicable.
   Sync every staged file, then atomically publish and sync a `prepared` journal and its parent
   directory before touching a target. JIT already has the necessary local pattern—`sync_all`,
   rename, then parent-directory `sync_all`—in heartbeat storage
   (`crates/jit/src/storage/heartbeat.rs:193-227`), whereas the shared atomic writer currently
   stops after rename (`crates/jit/src/storage/atomic_write.rs:39-42`). Rust documents
   [`File::sync_all`](https://doc.rust-lang.org/stable/std/fs/struct.File.html#method.sync_all) as
   the API to handle write/close errors rather than losing them at `Drop`; POSIX likewise notes
   that multi-entry changes may be only partly persistent after a crash unless explicitly
   synchronized ([POSIX general concepts](https://pubs.opengroup.org/onlinepubs/9799919799/basedefs/V1_chap04.html)).
4. **Publish reversibly.** For a replacement, atomically rename the original to its journaled
   backup name, then rename the fully prepared stage to the target. For a new file, use verified
   no-replace publication. Sync affected directories after directory-entry changes. Recheck the
   expected identity immediately before each action and fail closed on an unexpected occupant.
   Keeping the original inode as the backup makes rollback materially stronger than copying its
   bytes: restoring by rename retains its ordinary metadata and hard-link identity. The final
   stage already has its declared mode, so chmod is not a fallible late step. Rust's rename is a
   same-mount operation and has platform-specific replacement behavior
   ([`std::fs::rename`](https://doc.rust-lang.org/stable/std/fs/fn.rename.html)); therefore all
   stages, backups, and targets must preflight onto one supported filesystem/volume. The current
   artifact code already rejects a cross-filesystem stage/destination rather than copying
   (`crates/jit/src/storage/artifact_mutation.rs:219-242`,
   `crates/jit/src/storage/artifact_mutation.rs:275-287`).
5. **Make provenance and audit part of the transaction.** Treat the installed profile record and
   `events.jsonl` as planned targets, not post-success best-effort writes. For this one-time MVP
   operation, stage the exact current event-log bytes plus one typed profile-applied line and
   replace the log through the same backup protocol while holding its lock. This avoids a
   non-rollbackable late append: the current append path is independently fallible after opening,
   inspecting, repairing a torn tail, and writing the line
   (`crates/jit/src/storage/json.rs:828-858`). The record and event therefore roll back or commit
   together. A no-op reapplication creates neither.
6. **Commit, then clean.** After every final target and affected directory is synced, atomically
   change the journal to a durable `committed` decision. Only then delete backups/stages and,
   last, the journal. A cleanup failure after that decision is a committed success with a visible
   cleanup warning; reporting ordinary command failure would falsely promise that the transaction
   can still be rolled back. SQLite's documented rollback-journal protocol is useful prior art:
   the journal is made durable before changing the database, changes are synced before the commit
   point, and journal state decides whether recovery presents the old or new state
   ([SQLite atomic commit](https://www.sqlite.org/atomiccommit.html#_flushing_changes_to_mass_storage),
   [SQLite rollback](https://www.sqlite.org/atomiccommit.html#rollback)). This proposal applies
   the principle, not SQLite's page format.
7. **Recover before any later mutation.** Under `RepoWriteLock`, a `prepared` journal means roll
   back in reverse order; a `committed` journal means verify/finish the final state and cleanup.
   Each recovery action is identity-checked and idempotent. Keep the journal and return a typed
   recovery-required error if either rollback or roll-forward cannot prove the expected object.

### Why not backup + rollback alone?

| Property | Durable staging journal (chosen) | In-memory backup + rollback |
|---|---|---|
| Conflict before first write | Both can leave targets untouched after the locked recheck. | Same. |
| Ordinary I/O error/panic | Reverse actions from durable identities; return ordinary failure only after rollback succeeds. | Can compensate while the process remains alive, but bookkeeping may be incomplete and rollback failure has no durable recovery instruction. |
| Kill, crash, or power loss | A surviving journal selects rollback before commit or roll-forward/cleanup after commit. | Process memory disappears; a partial set and orphan backups have no authoritative decision or safe automatic interpretation. |
| Record/event consistency | Record and exact event-log final bytes are transaction participants. | A conventional late `append_event` can fail after files changed or survive while file rollback succeeds. |
| Durability cost | More I/O, directory syncs, journal parser/recovery tests, and retained stages/backups until commit. | Less code and I/O, but does not meet a recoverable crash contract. |

### Exact guarantee boundary

- **Ordinary returned failure:** all tracked targets are restored only if compensating rollback
  completes. If rollback itself fails, the command returns a distinct recovery-required error and
  must not claim “unchanged”; the durable journal remains.
- **Crash:** the on-disk set may be mixed until the next recovery. The guarantee is eventual
  all-old or all-new state after successful recovery, not instantaneous multi-file atomicity.
  POSIX atomic rename covers one directory-entry operation, not an entire set
  ([POSIX `rename`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html)).
- **Readers:** current read paths do not take `RepoWriteLock`
  (`crates/jit/src/storage/repo_lock.rs:26-30`), so a concurrent reader may observe a publication
  midpoint. The lock excludes cooperating writers and makes rollback ownership safe; it does not
  provide snapshot isolation. Achieving reader isolation would be a separate storage-wide change.
- **Uncooperative writers:** the guarantee covers JIT writers honoring the repository lock.
  Identity checks fail closed around external filesystem changes, but cannot make an arbitrary
  non-JIT writer transactional.
- **What “restored” means:** rename-aside restores original file bytes and inode-backed metadata;
  it does not promise unchanged directory timestamps, file `ctime`, or behavior of exotic
  filesystems. Profile-owned final identity should cover bytes and the manifest-declared
  executable semantic. ACL/xattr ownership is outside this MVP and must not be silently claimed.
- **Durability:** `sync_all`/directory sync can only provide the guarantees of the host filesystem
  and hardware. Even SQLite explicitly notes that storage can misreport synchronization
  ([SQLite durability caveat](https://www.sqlite.org/atomiccommit.html#_things_that_can_go_wrong)).
- **Cross-platform atomicity:** use same-volume `std::fs::rename` and verified no-replace
  publication only; reject unsupported/cross-volume targets in preflight. Windows can also reject
  replacement when sharing/access rules prevent it. There is no portable multi-file atomic
  primitive, so failure-injection tests are required on Unix and Windows rather than inferring
  equivalence from one platform.

## Decision 2: `include_dir` over one authored package tree

Choose **one checked-in package directory embedded at compile time with `include_dir`**, with
`manifest.toml` and its declared asset files as the only authored package content. A new direct
dependency on `include_dir` is justified; do not add `rust-embed`, a tar/zip package format, or a
custom build script for this single embedded MVP.

The binary-facing representation should be conceptually:

```text
profiles/jit-dogfood/
├── manifest.toml       # identity, compatibility, registries, projections, asset paths,
│                       # and executable=true/false
└── assets/...          # exact bytes only
```

`include_dir!` embeds a complete directory in the crate
([crate documentation](https://docs.rs/include_dir/latest/include_dir/macro.include_dir.html)),
so adding or removing an asset does not require editing a second Rust asset table. Parse the
embedded manifest into one Rust `ProfileManifest` type, recursively enumerate the embedded files,
and require a one-to-one declaration match: no missing declared asset and no undeclared embedded
asset. Keep paths sorted before validation, hashing, planning, and output.

### Derivations and ownership

- **Manifest/package facts:** ID, version, compatible JIT range, semantic contributions,
  projection operations, target paths, and `executable` are authored only in `manifest.toml`.
  This follows the repository's single-source rule (`AGENTS.md:177-178`) and keeps dogfood names
  out of engine policy (`AGENTS.md:176-178`).
- **Bytes:** come only from embedded asset files. The application path never reads the source
  checkout and never performs network access, satisfying the offline/git-free boundary
  (`dev/active/9b7b5f9c-mvp-scope-brief.md:15-20`).
- **Modes:** do not inherit build-host filesystem mode bits or embedding-library metadata. Derive
  `0644` versus executable `0755` semantics from the manifest boolean. Apply the execute bits on
  Unix and report “not applicable” on Windows, matching the existing platform split for hooks
  (`crates/jit/src/commands/hooks.rs:121-143`). Rust exposes granular mode bits only through its
  Unix-specific `PermissionsExt`
  ([official API](https://doc.rust-lang.org/std/os/unix/fs/trait.PermissionsExt.html)). Include the
  executable declaration in the package identity even though byte SHA-256 alone cannot see it.
- **Hashes:** compute SHA-256 from embedded bytes at runtime, with a canonical package digest over
  sorted `(path, executable, length, bytes)` tuples plus canonical manifest bytes. Store per-target
  byte hashes in the installed record. This uses the existing `sha2` dependency
  (`crates/jit/Cargo.toml:33-39`) and the repository already has a SHA-256 helper pattern
  (`crates/jit/src/snapshot.rs:221-225`); do not hand-author or check in a checksum inventory.
- **Schema:** derive `JsonSchema` on the same Rust manifest wire type used for deserialization and
  expose `schemars::schema_for!(ProfileManifest)` through `jit --schema`. The project already
  depends on schemars (`crates/jit/Cargo.toml:18-25`) and already generates its command schema in
  Rust (`crates/jit/src/schema.rs:160-202`). Schemars explicitly supports deriving a schema from a
  Rust type ([schemars documentation](https://docs.rs/schemars/latest/schemars/#basic-usage)). Do
  not embed a separately maintained `manifest.schema.json`; if docs need a materialized schema,
  generate it and verify drift in CI.
- **Offline package:** the embedded directory *is* the package available to `list`, `show`, and
  `apply`; there is no runtime archive extraction and no local-package discovery. Parse and fully
  validate it through the same generic manifest/package validator on first use, with a test that
  fails if the shipped embedded package is invalid.

### Alternatives considered

| Method | Assessment |
|---|---|
| `include_str!` / `include_bytes!` per file | Standard-library only and each file is truly compiled into the binary ([Rust `include_bytes!`](https://doc.rust-lang.org/core/macro.include_bytes.html)), as the two current hook constants demonstrate (`crates/jit/src/commands/hooks.rs:7-8`). It requires a second handwritten Rust inventory, so a newly added asset can exist in the package tree but not the binary. That violates the selected single-source property. |
| Custom `build.rs` scans the tree and generates an index/archive in `OUT_DIR` | Can fail the build on malformed content and needs no embedding proc macro, but it creates bespoke recursive traversal, escaping/code generation, deterministic ordering, Cargo change tracking, and generated-output tests. Cargo permits generated modules in `OUT_DIR` and directory `rerun-if-changed` tracking ([Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html#outputs-of-the-build-script), [change detection](https://doc.rust-lang.org/cargo/reference/build-scripts.html#change-detection)), but build scripts cannot use normal dependencies unless repeated as build dependencies. That complexity is not justified for one small data tree. |
| Generate and embed a tar/zip blob | A single `include_bytes!` is simple at runtime, and `tar` is already a normal dependency (`crates/jit/Cargo.toml:35-39`), but generating the blob still needs a build script or checked-in derived artifact. Archive headers introduce mode/timestamp/canonicalization questions and extraction adds a path-safety surface, while the engine only needs indexed immutable bytes. Reject. |
| `rust-embed` derive | Offers lookup and SHA-256 metadata, but its documented default reads files from the source filesystem in debug builds unless `debug-embed` is enabled; release builds embed them ([`RustEmbed` behavior](https://docs.rs/rust-embed/latest/rust_embed/trait.RustEmbed.html#tymethod.get)). That build-profile-dependent behavior is an avoidable offline-test hazard, and its extra hashing/metadata features would compete with manifest-owned mode and the existing `sha2` derivation. Reject. |
| `include_dir` (chosen) | Directly supplies recursive, compile-time directory embedding and immutable byte access with one package-tree inventory. Its proc macro adds a dependency and byte-string expansion can increase compile RAM for large trees ([compile-time caveat](https://docs.rs/include_dir/latest/include_dir/#compile-time-considerations)), but this MVP has one bounded text-asset package and explicitly excludes arbitrary local packages. Keep optional glob/metadata features off. |

### Dependency judgment and guardrails

The `include_dir` dependency is justified because recursive enumeration is the one capability that
standard `include_bytes!` lacks and that capability removes a correctness-sensitive duplicate
inventory. It is narrower and easier to test than a repository-owned code generator. Pin it in
`Cargo.lock`, enable no optional features, record license/supply-chain review, and add a size/count
test so accidental bulk inclusion fails loudly. Revisit the choice only if the embedded tree grows
large enough for the documented macro-expansion cost to become material; that is not permission to
add local packages or composition to this MVP.

## Resulting planning constraints

1. Treat publication/recovery as one storage subsystem that includes the profile record and typed
   event. Tests inject failure before and after every rename, sync, event-log replacement, commit
   decision, rollback action, and cleanup action.
2. Do not state “multi-file atomic” without the explicit boundaries above. The achievable MVP is
   conflict-free validated publication, exact tracked-state rollback on handled failure, and
   journal-driven crash recovery under one cooperating-writer lock.
3. Author dogfood package facts once in the embedded manifest/assets tree. Derive manifest schema,
   byte/mode hashes, installed provenance, list/show output, and any compatibility projection from
   that source.
4. Keep local package loading, composition, variables, upgrade, and removal absent from both the
   transaction format and embedding API.
