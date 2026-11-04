//! The storage-owned module through which jit starts git (`@/charter/D-21`).
//! Every process runs as `<program> -C <directory> …`, never in the process
//! working directory.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::thread;
use thiserror::Error;

use crate::domain::artifact_plan::{ArtifactVersion, CanonicalCommitOid};
use crate::repository_state::RepositoryTargetKind;
use crate::storage::validate_repo_relative_path;

/// Errors from running git.
#[derive(Debug, Clone, Error)]
pub enum GitError {
    /// The git executable could not be started.
    #[error("git is unavailable at {program}: {source}")]
    GitUnavailable {
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
    /// Git rejected the supplied revision under commit-peeling semantics.
    #[error("revision could not be resolved to a commit: {revision}: {stderr}")]
    RevisionNotFound { revision: String, stderr: String },
    /// Git returned output that is not a canonical SHA-1 or SHA-256 commit OID.
    #[error("git returned a non-canonical commit OID for {revision}: {output}")]
    NonCanonicalOid { revision: String, output: String },
    /// The historical path could not be read at the resolved commit.
    #[error("pinned artifact read failed for {path} at {revision}: {stderr}")]
    PinnedReadFailed {
        revision: String,
        path: String,
        stderr: String,
    },
    /// The artifact path violates the repository-relative storage contract.
    #[error("{0}")]
    InvalidPath(String),
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
        let output = match self.run(dir, ARGS) {
            Ok(output) if output.status.success() => output,
            Ok(_) | Err(GitError::GitUnavailable { .. }) => return Ok(None),
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

    /// Construct every process with an explicit checkout directory.
    fn command<'a>(&self, dir: &Path, args: impl IntoIterator<Item = &'a str>) -> Command {
        let mut command = Command::new(&self.program);
        command.arg("-C").arg(dir).args(args);
        command
    }

    fn unavailable(&self, source: io::Error) -> GitError {
        GitError::GitUnavailable {
            program: self.program.clone(),
            source: Arc::new(source),
        }
    }

    /// Run `<program> -C <dir> <args…>` to completion with its output captured.
    fn run<'a>(
        &self,
        dir: &Path,
        args: impl IntoIterator<Item = &'a str>,
    ) -> Result<Output, GitError> {
        self.command(dir, args)
            .output()
            .map_err(|source| self.unavailable(source))
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
    /// Returns [`GitError::GitUnavailable`] if git cannot start or
    /// [`GitError::UnexpectedOutput`] if successful output is not a canonical
    /// commit and branch pair.
    pub fn head(&self) -> Result<Option<Head>, GitError> {
        const ARGS: [&str; 4] = ["rev-parse", "HEAD", "--abbrev-ref", "HEAD"];
        let output = self.git.run(&self.worktree_root, ARGS)?;
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
        let output = self.git.run(&self.worktree_root, ARGS)?;
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
        let output = self.git.run(&self.worktree_root, args)?;
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

/// Maximum pinned paths passed to one `git ls-tree` process, keeping the
/// argument list well inside the platform limit.
const LS_TREE_PATHS_PER_PROCESS: usize = 1024;

/// Bytes read at one canonical historical commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedArtifactRead {
    version: ArtifactVersion,
    blob_oid: String,
    bytes: Vec<u8>,
}

/// No-follow classification of one target at a resolved historical commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedTargetRead {
    version: ArtifactVersion,
    target_kind: RepositoryTargetKind,
    object_oid: Option<String>,
    bytes: Option<Vec<u8>>,
    unavailable_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct TreeEntry {
    mode: String,
    object_type: String,
    object_oid: String,
}

impl PinnedTargetRead {
    /// This classification without its bytes.
    fn without_bytes(&self) -> Self {
        Self {
            bytes: None,
            ..self.clone()
        }
    }

    /// Canonical historical commit used for this classification.
    pub fn version(&self) -> &ArtifactVersion {
        &self.version
    }

    /// Captured filesystem kind at the named commit.
    pub fn target_kind(&self) -> RepositoryTargetKind {
        self.target_kind
    }

    /// Git object id for a present target.
    pub fn object_oid(&self) -> Option<&str> {
        self.object_oid.as_deref()
    }

    /// Exact bytes for an ordinary file target.
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }

    /// Stable reason when the target is missing at a resolved commit.
    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }
}

impl PinnedArtifactRead {
    /// Canonical pinned version associated with the bytes.
    pub fn version(&self) -> &ArtifactVersion {
        &self.version
    }

    /// True Git blob object id of the read content, so capture identity binds to
    /// Git's own object identity rather than a synthesized surrogate.
    pub fn blob_oid(&self) -> &str {
        &self.blob_oid
    }

    /// Byte-faithful blob content read from Git history.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl GitCheckout {
    /// Canonicalize `revision` with exact
    /// `git rev-parse --verify <revision>^{commit}` semantics.
    ///
    /// Both full SHA-1 and SHA-256 OIDs are accepted; abbreviated and symbolic
    /// inputs are returned only as Git's full canonical lowercase commit OID.
    pub fn resolve_commit(&self, revision: &str) -> Result<ArtifactVersion, GitError> {
        let peeled = format!("{revision}^{{commit}}");
        let output = self.git.run(
            &self.worktree_root,
            ["rev-parse", "--verify", peeled.as_str()],
        )?;
        if !output.status.success() {
            return Err(GitError::RevisionNotFound {
                revision: revision.to_string(),
                stderr: stderr(&output),
            });
        }

        canonical_version(revision, &String::from_utf8_lossy(&output.stdout))
    }

    /// Resolve `revision` and read `path` from that exact commit.
    ///
    /// The method never consults the working tree. Resolution failures and
    /// commit-tree/blob failures are returned to the caller as blockers.
    pub fn read_pinned_path(
        &self,
        revision: &str,
        path: &str,
    ) -> Result<PinnedArtifactRead, GitError> {
        validate_path(path)?;
        let version = self.resolve_commit(revision)?;
        let object = format!("{}:{path}", version.as_str());
        let oid_output = self.git.run(
            &self.worktree_root,
            ["rev-parse", "--verify", object.as_str()],
        )?;
        if !oid_output.status.success() {
            return Err(GitError::PinnedReadFailed {
                revision: revision.to_string(),
                path: path.to_string(),
                stderr: stderr(&oid_output),
            });
        }
        let blob_oid = String::from_utf8_lossy(&oid_output.stdout)
            .trim()
            .to_string();
        let output = self
            .git
            .run(&self.worktree_root, ["cat-file", "blob", object.as_str()])?;
        if !output.status.success() {
            return Err(GitError::PinnedReadFailed {
                revision: revision.to_string(),
                path: path.to_string(),
                stderr: stderr(&output),
            });
        }
        Ok(PinnedArtifactRead {
            version,
            blob_oid,
            bytes: output.stdout,
        })
    }

    /// Resolve and inspect a set of pinned targets with shared Git processes.
    ///
    /// A request in `kind_only` is classified from the commit tree alone: its
    /// read carries no bytes, and no blob is read for it.
    ///
    /// The supplied cache is owned by the caller and is replaced with only the
    /// successfully classified commit-and-path pairs used by this call. An empty
    /// request set spawns nothing and leaves the cache untouched, so captures
    /// that pin no documents do not evict evidence a later capture reuses.
    /// Revision names are always resolved again; the cache is intentionally
    /// keyed by the resulting full commit OID so moving symbolic revisions cannot
    /// serve stale evidence.
    pub fn inspect_pinned_targets(
        &self,
        requests: &BTreeSet<(String, String)>,
        kind_only: &BTreeSet<(String, String)>,
        cache: &mut BTreeMap<(String, String), PinnedTargetRead>,
    ) -> BTreeMap<(String, String), Result<PinnedTargetRead, GitError>> {
        if requests.is_empty() {
            return BTreeMap::new();
        }
        let previous_cache = std::mem::take(cache);
        let mut next_cache = BTreeMap::new();
        let mut results: BTreeMap<(String, String), Result<PinnedTargetRead, GitError>> =
            BTreeMap::new();
        let valid_requests = requests
            .iter()
            .filter(|request| match validate_path(&request.1) {
                Ok(()) => true,
                Err(error) => {
                    results.insert((*request).clone(), Err(error));
                    false
                }
            })
            .collect::<Vec<_>>();
        let revisions = valid_requests
            .iter()
            .map(|(revision, _)| revision.clone())
            .collect::<BTreeSet<_>>();
        let resolutions = self.resolve_commits(&revisions);

        // Resolve every request first so an invalid revision can never prevent
        // valid revisions in the same batch from being inspected.
        let mut uncached =
            BTreeMap::<String, BTreeMap<String, Vec<(String, String, ArtifactVersion)>>>::new();
        valid_requests.into_iter().for_each(|request| {
            let (revision, path) = request;
            let wants_kind = kind_only.contains(request);
            match resolutions.get(revision) {
                Some(Ok(version)) => {
                    let cache_key = (version.as_str().to_string(), path.clone());
                    let cached = previous_cache.get(&cache_key).filter(|read| {
                        wants_kind || read.bytes.is_some() || !read.target_kind.is_file()
                    });
                    if let Some(read) = cached {
                        let answer = if wants_kind {
                            read.without_bytes()
                        } else {
                            read.clone()
                        };
                        results.insert(request.clone(), Ok(answer));
                        next_cache.insert(cache_key, read.clone());
                    } else {
                        uncached
                            .entry(version.as_str().to_string())
                            .or_default()
                            .entry(path.clone())
                            .or_default()
                            .push((revision.clone(), path.clone(), version.clone()));
                    }
                }
                Some(Err(error)) => {
                    results.insert(request.clone(), Err(error.clone()));
                }
                None => {
                    results.insert(
                        request.clone(),
                        Err(GitError::RevisionNotFound {
                            revision: revision.clone(),
                            stderr: "revision was not resolved".into(),
                        }),
                    );
                }
            }
        });

        let mut pending_files =
            BTreeMap::<String, Vec<(String, String, String, PinnedTargetRead)>>::new();
        uncached.into_iter().for_each(|(commit, paths)| {
            let entries = match self.list_pinned_tree_entries(&commit, paths.keys()) {
                Ok(Ok(entries)) => entries,
                Err(error) => {
                    paths.values().flatten().for_each(|(revision, path, _)| {
                        results.insert((revision.clone(), path.clone()), Err(error.clone()));
                    });
                    return;
                }
                Ok(Err(reason)) => {
                    paths.values().flatten().for_each(|(revision, path, _)| {
                        results.insert(
                            (revision.clone(), path.clone()),
                            Err(GitError::PinnedReadFailed {
                                revision: revision.clone(),
                                path: path.clone(),
                                stderr: reason.clone(),
                            }),
                        );
                    });
                    return;
                }
            };
            paths.into_iter().for_each(|(path, requests)| {
                let Some(first_request) = requests.first() else {
                    return;
                };
                let target = entries.get(path.as_bytes()).map(|entry| {
                    let target_kind = match (entry.mode.as_str(), entry.object_type.as_str()) {
                        ("100644" | "100755", "blob") => RepositoryTargetKind::File,
                        ("040000", "tree") => RepositoryTargetKind::Directory,
                        _ => RepositoryTargetKind::Unsupported,
                    };
                    PinnedTargetRead {
                        version: first_request.2.clone(),
                        target_kind,
                        object_oid: Some(entry.object_oid.clone()),
                        bytes: None,
                        unavailable_reason: None,
                    }
                });
                let target = target.unwrap_or_else(|| PinnedTargetRead {
                    version: first_request.2.clone(),
                    target_kind: RepositoryTargetKind::Missing,
                    object_oid: None,
                    bytes: None,
                    unavailable_reason: Some("not found in commit tree".into()),
                });
                if target.target_kind.is_file() {
                    let (kinds, wholes): (Vec<_>, Vec<_>) =
                        requests.into_iter().partition(|(revision, path, _)| {
                            kind_only.contains(&(revision.clone(), path.clone()))
                        });
                    kinds.into_iter().for_each(|(revision, path, _)| {
                        results.insert((revision, path.clone()), Ok(target.clone()));
                        next_cache.insert((commit.clone(), path), target.clone());
                    });
                    if let Some(object_oid) =
                        target.object_oid.clone().filter(|_| !wholes.is_empty())
                    {
                        pending_files.entry(object_oid).or_default().extend(
                            wholes.into_iter().map(|(revision, path, _)| {
                                (revision, path, commit.clone(), target.clone())
                            }),
                        );
                    }
                } else {
                    requests.into_iter().for_each(|(revision, path, _)| {
                        results.insert((revision, path.clone()), Ok(target.clone()));
                        next_cache.insert((commit.clone(), path), target.clone());
                    });
                }
            });
        });

        if !pending_files.is_empty() {
            let oids = pending_files.keys().cloned().collect::<BTreeSet<_>>();
            match self.read_blob_batch(&oids) {
                Ok(blob_results) => pending_files.into_iter().for_each(|(oid, requests)| {
                    requests
                        .into_iter()
                        .for_each(|(revision, path, commit, mut target)| {
                            match blob_results.get(&oid) {
                                Some(Ok(bytes)) => {
                                    target.bytes = Some(bytes.clone());
                                    results.insert((revision, path.clone()), Ok(target.clone()));
                                    next_cache.insert((commit, path), target);
                                }
                                Some(Err(reason)) => {
                                    results.insert(
                                        (revision.clone(), path.clone()),
                                        Err(GitError::PinnedReadFailed {
                                            revision,
                                            path,
                                            stderr: reason.clone(),
                                        }),
                                    );
                                }
                                None => {
                                    results.insert(
                                        (revision.clone(), path.clone()),
                                        Err(GitError::PinnedReadFailed {
                                            revision,
                                            path,
                                            stderr:
                                                "git cat-file --batch omitted the requested object"
                                                    .into(),
                                        }),
                                    );
                                }
                            }
                        });
                }),
                Err(error) => pending_files.into_iter().for_each(|(_, requests)| {
                    requests.into_iter().for_each(|(revision, path, _, _)| {
                        results.insert((revision.clone(), path.clone()), Err(error.clone()));
                    });
                }),
            }
        }

        *cache = next_cache;
        results
    }

    /// List the tree entries of `commit` at the requested `paths`, keyed by
    /// path bytes.
    ///
    /// The paths are passed to `git ls-tree` as literal pathspecs, so the
    /// output scales with the requested paths rather than with the commit
    /// tree. `-t` keeps a requested directory's own entry when a path beneath
    /// it is also requested. The listing may include unrequested siblings;
    /// callers look entries up by exact path. Paths are split across
    /// processes in chunks of [`LS_TREE_PATHS_PER_PROCESS`] to bound the
    /// argument list. The outer error is a spawn failure; the inner one is a
    /// git failure or malformed output.
    fn list_pinned_tree_entries<'a>(
        &self,
        commit: &str,
        paths: impl Iterator<Item = &'a String>,
    ) -> Result<Result<BTreeMap<Vec<u8>, TreeEntry>, String>, GitError> {
        let paths = paths.map(String::as_str).collect::<Vec<_>>();
        paths
            .chunks(LS_TREE_PATHS_PER_PROCESS)
            .try_fold(Ok(BTreeMap::new()), |listed, chunk| {
                let Ok(mut listed) = listed else {
                    return Ok(listed);
                };
                let arguments = [
                    "--literal-pathspecs",
                    "ls-tree",
                    "-t",
                    "-z",
                    "--full-tree",
                    commit,
                    "--",
                ]
                .into_iter()
                .chain(chunk.iter().copied());
                let output = self.git.run(&self.worktree_root, arguments)?;
                if !output.status.success() {
                    return Ok(Err(stderr(&output)));
                }
                Ok(parse_pinned_tree_entries(&output.stdout).map(|entries| {
                    listed.extend(entries);
                    listed
                }))
            })
    }

    fn resolve_commits(
        &self,
        revisions: &BTreeSet<String>,
    ) -> BTreeMap<String, Result<ArtifactVersion, GitError>> {
        if revisions.is_empty() {
            return BTreeMap::new();
        }
        let peeled = revisions
            .iter()
            .map(|revision| format!("{revision}^{{commit}}"))
            .collect::<Vec<_>>();
        let mut arguments = vec!["rev-parse", "--end-of-options"];
        arguments.extend(peeled.iter().map(String::as_str));
        let output = match self.git.run(&self.worktree_root, arguments) {
            Ok(output) if output.status.success() => output,
            Ok(_) => return self.resolve_commits_individually(revisions),
            Err(error) => {
                return revisions
                    .iter()
                    .map(|revision| (revision.clone(), Err(error.clone())))
                    .collect()
            }
        };
        let lines = output
            .stdout
            .split(|byte| *byte == b'\n')
            // Git 2.55 echoes the end-of-options marker once when rev-parse
            // is used without --verify. It is not a resolved revision.
            .filter(|line| !line.is_empty() && *line != b"--end-of-options")
            .collect::<Vec<_>>();
        if lines.len() != revisions.len() {
            return self.resolve_commits_individually(revisions);
        }
        revisions
            .iter()
            .zip(lines)
            .map(|(revision, line)| {
                (
                    revision.clone(),
                    canonical_version(revision, &String::from_utf8_lossy(line)),
                )
            })
            .collect()
    }

    fn resolve_commits_individually(
        &self,
        revisions: &BTreeSet<String>,
    ) -> BTreeMap<String, Result<ArtifactVersion, GitError>> {
        revisions
            .iter()
            .map(|revision| (revision.clone(), self.resolve_commit(revision)))
            .collect()
    }

    fn read_blob_batch(
        &self,
        oids: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, Result<Vec<u8>, String>>, GitError> {
        let mut child = self
            .git
            .command(&self.worktree_root, ["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| self.git.unavailable(source))?;
        let Some(mut stdin) = child.stdin.take() else {
            return Ok(oids
                .iter()
                .map(|oid| {
                    (
                        oid.clone(),
                        Err("git cat-file --batch did not expose stdin".into()),
                    )
                })
                .collect());
        };
        let write_oids = oids.iter().cloned().collect::<Vec<_>>();
        let writer = thread::spawn(move || {
            let result = write_oids
                .iter()
                .try_for_each(|oid| writeln!(stdin, "{oid}"));
            drop(stdin);
            result
        });
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.wait();
            let _ = writer.join();
            return Ok(oids
                .iter()
                .map(|oid| {
                    (
                        oid.clone(),
                        Err("git cat-file --batch did not expose stdout".into()),
                    )
                })
                .collect());
        };
        let stderr_reader = child.stderr.take().map(|mut stderr_pipe| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                let result = stderr_pipe.read_to_end(&mut bytes);
                result.map(|_| bytes)
            })
        });
        let mut bytes = Vec::new();
        let read_result = stdout.read_to_end(&mut bytes).map(|_| ());
        let wait_result = child.wait();
        let writer_result = writer
            .join()
            .map_err(|_| io::Error::other("git cat-file input writer panicked"))
            .and_then(|result| result);
        let stderr_bytes = stderr_reader
            .map(|reader| {
                reader
                    .join()
                    .map_err(|_| io::Error::other("git cat-file stderr reader panicked"))
                    .and_then(|result| result)
            })
            .transpose()
            .map_err(|error| GitError::PinnedReadFailed {
                revision: "batch".into(),
                path: "<blobs>".into(),
                stderr: error.to_string(),
            })?
            .unwrap_or_default();
        if let Err(error) = read_result.or(writer_result) {
            return Ok(oids
                .iter()
                .map(|oid| (oid.clone(), Err(error.to_string())))
                .collect());
        }
        let status = wait_result.map_err(|error| GitError::PinnedReadFailed {
            revision: "batch".into(),
            path: "<blobs>".into(),
            stderr: error.to_string(),
        })?;
        if !status.success() {
            let reason = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
            return Ok(oids
                .iter()
                .map(|oid| (oid.clone(), Err(reason.clone())))
                .collect());
        }
        parse_cat_file_batch(&bytes, oids).map_err(|reason| GitError::PinnedReadFailed {
            revision: "batch".into(),
            path: "<blobs>".into(),
            stderr: reason,
        })
    }
}

fn parse_pinned_tree_entries(output: &[u8]) -> Result<Vec<(Vec<u8>, TreeEntry)>, String> {
    output
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let Some(tab_index) = entry.iter().position(|byte| *byte == b'\t') else {
                return Err("git ls-tree entry lacks a path separator".into());
            };
            let (header, path_with_separator) = entry.split_at(tab_index);
            let path = path_with_separator[1..].to_vec();
            let fields = header.split(|byte| *byte == b' ').collect::<Vec<_>>();
            let [mode, object_type, oid] = fields.as_slice() else {
                return Err("git ls-tree entry has an invalid header".into());
            };
            let decode = |field: &[u8]| {
                std::str::from_utf8(field)
                    .map(str::to_owned)
                    .map_err(|_| "git ls-tree entry is not UTF-8".to_string())
            };
            Ok((
                path,
                TreeEntry {
                    mode: decode(mode)?,
                    object_type: decode(object_type)?,
                    object_oid: decode(oid)?,
                },
            ))
        })
        .collect()
}

fn parse_cat_file_batch(
    output: &[u8],
    requested_oids: &BTreeSet<String>,
) -> Result<BTreeMap<String, Result<Vec<u8>, String>>, String> {
    let mut responses = BTreeMap::new();
    let mut offset = 0;
    while offset < output.len() {
        let Some(header_length) = output[offset..].iter().position(|byte| *byte == b'\n') else {
            return Err("git cat-file --batch response lacks a header terminator".into());
        };
        let header_end = offset + header_length;
        let fields = output[offset..header_end]
            .split(|byte| *byte == b' ')
            .collect::<Vec<_>>();
        let [oid, object_type, size] = fields.as_slice() else {
            return Err("git cat-file --batch response has an invalid header".into());
        };
        let oid = std::str::from_utf8(oid)
            .map(str::to_owned)
            .map_err(|_| "git cat-file --batch response has a non-UTF-8 OID".to_string())?;
        offset = header_end + 1;
        if *object_type == b"missing" {
            responses.insert(
                oid,
                Err("git cat-file --batch reported a missing object".into()),
            );
            continue;
        }
        if *object_type != b"blob" {
            return Err("git cat-file --batch returned a non-blob object".into());
        }
        let size = std::str::from_utf8(size)
            .map_err(|_| "git cat-file --batch response has an invalid size".to_string())?
            .parse::<usize>()
            .map_err(|_| "git cat-file --batch response has an invalid size".to_string())?;
        let Some(content_end) = offset.checked_add(size) else {
            return Err("git cat-file --batch response size overflows".into());
        };
        if content_end >= output.len() || output[content_end] != b'\n' {
            return Err("git cat-file --batch response lacks its content terminator".into());
        }
        responses.insert(oid, Ok(output[offset..content_end].to_vec()));
        offset = content_end + 1;
    }
    if requested_oids
        .iter()
        .any(|oid| !responses.contains_key(oid))
    {
        return Err("git cat-file --batch returned fewer responses than requested".into());
    }
    Ok(responses)
}

fn validate_path(path: &str) -> Result<(), GitError> {
    validate_repo_relative_path(path).map_err(|error| GitError::InvalidPath(error.to_string()))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn canonical_version(revision: &str, output: &str) -> Result<ArtifactVersion, GitError> {
    let oid = output.trim().to_string();
    ArtifactVersion::pinned(oid.clone()).map_err(|_| GitError::NonCanonicalOid {
        revision: revision.to_string(),
        output: oid,
    })
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
            .run(dir.path(), ["version"])
            .unwrap_err();

        assert!(
            matches!(&error, GitError::GitUnavailable { program: named, .. } if named == &program)
        );
        assert!(error.to_string().contains(program.to_str().unwrap()));
    }

    #[test]
    fn test_canonical_version_accepts_full_sha1_and_sha256_git_output() {
        let sha1 = "0123456789abcdef0123456789abcdef01234567";
        let sha256 = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

        assert_eq!(canonical_version("HEAD", sha1).unwrap().as_str(), sha1);
        assert_eq!(
            canonical_version("HEAD", &format!("{sha256}\n"))
                .unwrap()
                .as_str(),
            sha256
        );
    }

    #[test]
    fn test_canonical_version_rejects_abbreviated_or_noncanonical_git_output() {
        assert!(matches!(
            canonical_version("HEAD", "01234567"),
            Err(GitError::NonCanonicalOid { .. })
        ));
        assert!(matches!(
            canonical_version("HEAD", "0123456789ABCDEF0123456789ABCDEF01234567"),
            Err(GitError::NonCanonicalOid { .. })
        ));
    }
}
