# Installed nonblocking-read verification

Issue: 6326532e. Decision: @/charter/D-25.

The reviewed change merged as `abaa6bdf8`. The merged `cargo-ci` gate passed stably in run `22112c9d`; evidence is recorded by `0cd45c1c6`. The independent review passed in run `d17b5a0e` with no findings. Documentation mechanical checks passed on the reviewed branch, including citations and projections.

On 2026-11-05, `./scripts/install-jit.sh --force --locked` installed `/home/vkaskivuo/.cargo/bin/jit` from the merged tree. Its schema reports exit 12 for a repository that changed or is busy. The configured server tests cover the shared `RETRYABLE_CONFLICT` to HTTP 503 mapping.

An isolated scratch repository was initialized, and its `.jit/.repo-write.lock` was held exclusively for both reads. `timeout 3s jit issue list --json` returned successfully on stable data. After creating the existing test fixture `.jit/tmp/redo.record` containing `pending`, the same command returned exit 12 and JSON error code `RETRYABLE_CONFLICT` within the timeout; the record still contained `pending`. The scratch repository was removed afterward. This verifies that the installed read neither waits for the writer lock nor replays a pending record; explicit recovery and mutation recovery retain their own behavior.
