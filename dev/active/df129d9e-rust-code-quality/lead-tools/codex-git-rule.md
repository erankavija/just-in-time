
# Git rule for Codex workers (overrides any merge instruction above)

Do not run `git merge`, `git commit`, `git add`, `git checkout`, `git restore`, `git stash`, `git reset` or any other git command that writes. Your sandbox makes git metadata read-only, and workarounds corrupt the branch. Read-only git (`status`, `diff`, `log`, `show`, `grep`) is fine. Leave all changes uncommitted; the lead merges main into your branch and commits after review.

# Build rule for Codex workers

Prefix every cargo command with `CARGO_INCREMENTAL=0` (for example `CARGO_INCREMENTAL=0 cargo test -p jit ...`). Your sandboxed shell may not inherit the exported variable, and incremental state fills the shared disk.
