
# Git rule for Codex workers (overrides any merge instruction above)

Do not run `git merge`, `git commit`, `git add`, `git checkout`, `git restore`, `git stash`, `git reset` or any other git command that writes. Your sandbox makes git metadata read-only, and workarounds corrupt the branch. Read-only git (`status`, `diff`, `log`, `show`, `grep`) is fine. Leave all changes uncommitted; the lead merges main into your branch and commits after review.
