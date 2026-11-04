//! The storage-owned module through which jit starts git (`@/charter/D-21`).
//! Every process runs as `<program> -C <directory> …`, never in the process
//! working directory.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use thiserror::Error;

use crate::domain::artifact_plan::CanonicalCommitOid;

/// Errors from running git.
#[derive(Debug, Clone, Error)]
pub enum GitError {
    /// The git executable could not be started.
    #[error("git is unavailable at {program}: {source}")]
    Unavailable {
        program: PathBuf,
        #[source]
        source: Arc<io::Error>,
    },
    /// Git succeeded but its output does not have the requested shape.
    #[error("unexpected output from git {command}: {output}")]
    UnexpectedOutput { command: String, output: String },
    /// A path git reported could not be resolved.
    #[error("cannot resolve path {path} reported by git: {source}")]
    UnresolvedPath {
        path: PathBuf,
        #[source]
        source: Arc<io::Error>,
    },
}

/// The git executable; the default is the bare name `git`, looked up on
/// `PATH` when a process starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Git {
    program: PathBuf,
}

impl Default for Git {
    fn default() -> Self {
        Self::with_program("git")
    }
}

/// The layout facts of a directory inside a git checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbedCheckout {
    /// The common directory shared by every checkout of the repository.
    pub common_dir: PathBuf,
    /// The top-level directory of the probed checkout.
    pub worktree_root: PathBuf,
    /// The checkout's own git directory: the common directory for a primary
    /// checkout, its per-worktree directory for a linked one.
    pub git_dir: PathBuf,
}

impl Git {
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Classify `dir` with one git process.
    ///
    /// Returns `None` outside a checkout, inside a `.git` directory, and when
    /// the executable cannot start. A relative common directory is resolved
    /// against `dir`.
    ///
    /// # Errors
    ///
    /// Returns an error when git reports a checkout whose paths cannot be read
    /// or resolved.
    pub fn probe(&self, dir: &Path) -> Result<Option<ProbedCheckout>, GitError> {
        const ARGS: [&str; 4] = [
            "rev-parse",
            "--git-common-dir",
            "--show-toplevel",
            "--absolute-git-dir",
        ];
        let output = match self.run(dir, &ARGS) {
            Ok(output) if output.status.success() => output,
            Ok(_) | Err(GitError::Unavailable { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let unexpected = || GitError::UnexpectedOutput {
            command: ARGS.join(" "),
            output: String::from_utf8_lossy(&output.stdout).into_owned(),
        };
        let stdout = std::str::from_utf8(&output.stdout).map_err(|_| unexpected())?;
        let (common_dir, worktree_root, git_dir) = match stdout.lines().collect::<Vec<_>>()[..] {
            [common_dir, worktree_root, git_dir] => {
                (Path::new(common_dir), Path::new(worktree_root), git_dir)
            }
            _ => return Err(unexpected()),
        };
        let common_dir = if common_dir.is_absolute() {
            common_dir.to_path_buf()
        } else {
            let joined = dir.join(common_dir);
            joined
                .canonicalize()
                .map_err(|source| GitError::UnresolvedPath {
                    path: joined,
                    source: Arc::new(source),
                })?
        };
        Ok(Some(ProbedCheckout {
            common_dir,
            worktree_root: worktree_root.to_path_buf(),
            git_dir: PathBuf::from(git_dir),
        }))
    }

    /// Run `<program> -C <dir> <args…>` to completion with its output captured.
    fn run(&self, dir: &Path, args: &[&str]) -> Result<Output, GitError> {
        Command::new(&self.program)
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .map_err(|source| GitError::Unavailable {
                program: self.program.clone(),
                source: Arc::new(source),
            })
    }
}

/// A git checkout and the executable that operates on it; carries the
/// module's git operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCheckout {
    git: Git,
    worktree_root: PathBuf,
}

impl GitCheckout {
    pub fn new(git: Git, worktree_root: PathBuf) -> Self {
        Self { git, worktree_root }
    }

    // Temporary @/inv/convention-convergence exception: branch lookups converge
    // here when 732f7afb, 9de4c09c, f6eaba42, 3f8da8c4 and c140921c land.
    /// Read the checked-out commit and branch with one git process.
    ///
    /// An unborn branch has no head. Detached checkouts have a commit and no
    /// branch name.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::Unavailable`] if git cannot start or
    /// [`GitError::UnexpectedOutput`] if successful output is not a canonical
    /// commit and branch pair.
    pub fn head(&self) -> Result<Option<Head>, GitError> {
        const ARGS: [&str; 4] = ["rev-parse", "HEAD", "--abbrev-ref", "HEAD"];
        let output = self.git.run(&self.worktree_root, &ARGS)?;
        if !output.status.success() {
            return Ok(None);
        }

        let unexpected = || GitError::UnexpectedOutput {
            command: ARGS.join(" "),
            output: String::from_utf8_lossy(&output.stdout).into_owned(),
        };
        let stdout = std::str::from_utf8(&output.stdout).map_err(|_| unexpected())?;
        let (commit, branch) = match stdout.lines().collect::<Vec<_>>().as_slice() {
            [commit, branch] => (*commit, *branch),
            _ => return Err(unexpected()),
        };
        let commit = CanonicalCommitOid::parse(commit).map_err(|_| unexpected())?;
        let branch = match branch {
            "HEAD" => None,
            "" => return Err(unexpected()),
            branch => Some(branch.to_owned()),
        };
        Ok(Some(Head { commit, branch }))
    }

    // Temporary @/inv/convention-convergence exception: the parsers in
    // parse_git_worktree_porcelain and recorded_worktree_ids converge here
    // when e3c38fea and f649c144 land.
    /// List every worktree, including detached and bare entries, with one git
    /// process. Branch names omit the `refs/heads/` prefix.
    ///
    /// # Errors
    ///
    /// Returns an error when git cannot start, fails, or reports malformed or
    /// non-UTF-8 output.
    pub fn list_worktrees(&self) -> Result<Vec<Worktree>, GitError> {
        const ARGS: [&str; 4] = ["worktree", "list", "--porcelain", "-z"];
        let output = self.git.run(&self.worktree_root, &ARGS)?;
        let unexpected = |output: &str| GitError::UnexpectedOutput {
            command: ARGS.join(" "),
            output: output.to_owned(),
        };
        if !output.status.success() {
            return Err(unexpected(&String::from_utf8_lossy(&output.stderr)));
        }
        let stdout = std::str::from_utf8(&output.stdout)
            .map_err(|_| unexpected(&String::from_utf8_lossy(&output.stdout)))?;
        stdout
            .split("\0\0")
            .filter(|block| !block.is_empty())
            .map(|block| {
                let mut fields = block.split('\0');
                let path = fields
                    .next()
                    .and_then(|line| line.strip_prefix("worktree "))
                    .filter(|path| !path.is_empty())
                    .ok_or_else(|| unexpected(block))?;
                let branch = fields
                    .find_map(|line| line.strip_prefix("branch refs/heads/"))
                    .map(str::to_owned);
                Ok(Worktree {
                    path: PathBuf::from(path),
                    branch,
                })
            })
            .collect()
    }

    /// The checkout's top level, which [`Self::dirty_paths`] reports relative to.
    pub fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }

    /// Repository-relative paths that differ from `HEAD`: modified, staged,
    /// deleted, renamed (both names) or untracked, with ignored files
    /// excluded. One git process.
    ///
    /// # Errors
    ///
    /// Returns an error when git cannot start, fails, or reports a path that
    /// is not UTF-8.
    pub fn dirty_paths(&self) -> Result<Vec<PathBuf>, GitError> {
        let args = ["status", "--porcelain=v1", "-z", "--untracked-files=all"];
        let output = self.git.run(&self.worktree_root, &args)?;
        let unexpected = |output: String| GitError::UnexpectedOutput {
            command: args.join(" "),
            output,
        };
        if !output.status.success() {
            return Err(unexpected(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        let stdout = std::str::from_utf8(&output.stdout)
            .map_err(|_| unexpected(String::from_utf8_lossy(&output.stdout).into_owned()))?;
        let mut fields = stdout.split('\0').filter(|field| !field.is_empty());
        let mut paths = Vec::new();
        while let Some(entry) = fields.next() {
            let (status, path) = entry
                .split_at_checked(3)
                .ok_or_else(|| unexpected(entry.to_owned()))?;
            paths.push(PathBuf::from(path));
            if status
                .bytes()
                .take(2)
                .any(|code| matches!(code, b'R' | b'C'))
            {
                let original = fields.next().ok_or_else(|| unexpected(entry.to_owned()))?;
                paths.push(PathBuf::from(original));
            }
        }
        Ok(paths)
    }
}

/// A repository worktree and its checked-out branch, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    path: PathBuf,
    branch: Option<String>,
}

impl Worktree {
    /// The absolute path reported by git.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The branch name, or `None` for detached and bare worktrees.
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }
}

/// Commit and branch identity for a checkout with a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    commit: CanonicalCommitOid,
    branch: Option<String>,
}

impl Head {
    /// The full canonical commit OID.
    pub fn commit(&self) -> &CanonicalCommitOid {
        &self.commit
    }

    /// The branch name, or `None` when `HEAD` is detached.
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// The branch name, or `HEAD` when the checkout is detached.
    pub fn branch_label(&self) -> &str {
        self.branch().unwrap_or("HEAD")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_unstartable_program_is_unavailable_naming_program() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("missing-git");

        let error = Git::with_program(&program)
            .run(dir.path(), &["version"])
            .unwrap_err();

        assert!(
            matches!(&error, GitError::Unavailable { program: named, .. } if named == &program)
        );
        assert!(error.to_string().contains(program.to_str().unwrap()));
    }
}
