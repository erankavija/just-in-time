# Capture CPU benchmark

Before-and-after evidence for REQ-05. The fix scopes pinned tree listings to the requested paths, removes captured bytes from plan-hash serialization, validates each pinned request once, and reuses pinned evidence while its request resolves to the same commit and object.

## Method

The repository is a `git clone --shared` of an adopter repository at `3da0ebbb3` on tmpfs, with its `.jit/` copied in: 876 issues, 48k tracked files, 1687 document references across 42 distinct pinned revisions, and 129 MB of pinned blobs. A `git` wrapper ahead of the real `git` on `PATH` logs every invocation. Each row is the median of three runs of one `jit issue update <id> --add-label` or `--remove-label` process, timed as wall clock.

"before" is the installed jit 1.0.0 binary built at `a297f5c55`. "after" is the release build of this change.

## Results

| scenario | before: wall | after: wall | before: `ls-tree` output | after: `ls-tree` output |
| --- | ---: | ---: | ---: | ---: |
| label update that changes the issue | 6473 ms | 1828 ms | 42 full-tree listings, ~9 MB each | 43 path-scoped listings |
| idempotent label update | 4040 ms | 1402 ms | same | same |

Both builds spawn 48-49 git processes. The after build splits one revision's more than 1024 requested paths across two `ls-tree` processes.

A CPU profile of the before binary attributes the time to four sources:

- parsing the full-tree listings;
- serializing every captured file's bytes as JSON number arrays into the plan hash;
- revalidating the whole capture declaration for each pinned request, which is quadratic in the pinned-request count;
- hashing the 129 MB of pinned blobs twice or three times per capture, at about seven captures per mutation.

## Remaining cost

After the fix, most of the remaining 1.8 s goes to hashing about 116 MB of unpinned worktree documents. Every capture hashes each document once when reading it and once more when closing the image, and one mutation captures about seven times. Worktree bytes can change between captures, so this cost needs a different remedy than the commit-keyed reuse applied to pinned evidence.
