//! The storage-owned module through which jit starts git (`@/charter/D-21`).
//! Every process runs as `<program> -C <directory> …`, never in the process
//! working directory.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Output, Stdio};
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
    /// Git failed or returned output outside the requested command contract.
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
    /// A path or object name violates its input contract.
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

/// The relationship between a checkout's HEAD and a requested base commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseComparison {
    /// Both commits resolve and share history.
    Compared {
        /// The nearest shared ancestor of HEAD and the base commit.
        merge_base: CanonicalCommitOid,
        /// The base revision peeled to a full commit OID.
        base_commit: CanonicalCommitOid,
    },
    /// The base revision cannot be peeled to a commit.
    BaseUnresolved,
    /// Both commits resolve but share no history.
    NoMergeBase,
}

impl GitCheckout {
    /// Construct operations for a checkout already recognized by Git.
    pub(super) fn new(git: Git, worktree_root: PathBuf) -> Self {
        Self { git, worktree_root }
    }

    /// Construct a checkout with an explicit executable for tests.
    #[cfg(feature = "test-support")]
    pub fn for_test(git: Git, worktree_root: PathBuf) -> Self {
        Self::new(git, worktree_root)
    }

    // Temporary @/inv/convention-convergence exception: branch lookups converge
    // here when 732f7afb, 9de4c09c, f6eaba42, 3f8da8c4 and c140921c land.
    /// Read the checked-out commit and branch with at most two git processes.
    ///
    /// Validates the captured commit without history traversal or fetching.
    /// Only a confirmed unborn branch has no head; detached heads have no branch.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::GitUnavailable`] if git cannot start, or
    /// [`GitError::UnexpectedOutput`] for command failures or invalid HEAD evidence.
    pub fn head(&self) -> Result<Option<Head>, GitError> {
        const IDENTITY: [&str; 6] = [
            "--no-replace-objects",
            "rev-parse",
            "--revs-only",
            "HEAD",
            "--symbolic-full-name",
            "HEAD",
        ];
        let output = self.git.run(&self.worktree_root, IDENTITY)?;
        let head = parse_head_output(&IDENTITY, &output, |stdout| {
            if stdout.is_empty() {
                Some(None)
            } else {
                parse_head_identity(stdout).map(Some)
            }
        })?;

        match head {
            Some(head) => {
                // Replacements and commit graphs can mask a missing original object.
                let args = [
                    "--no-replace-objects",
                    "-c",
                    "core.commitGraph=false",
                    "rev-list",
                    "--no-walk",
                    "--missing=print",
                    head.commit.as_str(),
                    "--",
                ];
                let output = self.git.run(&self.worktree_root, args)?;
                parse_head_output(&args, &output, |stdout| {
                    (stdout.strip_suffix('\n') == Some(head.commit.as_str())).then_some(())
                })?;
                Ok(Some(head))
            }
            None => {
                const UNBORN: [&str; 4] =
                    ["--no-replace-objects", "symbolic-ref", "--quiet", "HEAD"];
                let output = self.git.run(&self.worktree_root, UNBORN)?;
                parse_head_output(&UNBORN, &output, |stdout| {
                    parse_head_branch(stdout.strip_suffix('\n')?).map(|_| ())
                })?;
                Ok(None)
            }
        }
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

    // Temporary @/inv/convention-convergence exception: claim release's actor
    // fallback still queries user.name directly until ea4b7363.
    /// Read the configured user name, if this checkout has one.
    ///
    /// # Errors
    ///
    /// Returns an error when git cannot start, fails for a reason other than
    /// the name being unset, or reports non-UTF-8 output.
    pub fn user_name(&self) -> Result<Option<String>, GitError> {
        const ARGS: [&str; 2] = ["config", "user.name"];
        let output = self.git.run(&self.worktree_root, ARGS)?;
        if !output.status.success() {
            if output.status.code() == Some(1) {
                return Ok(None);
            }
            return Err(GitError::UnexpectedOutput {
                command: ARGS.join(" "),
                output: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        let name = std::str::from_utf8(&output.stdout).map_err(|_| GitError::UnexpectedOutput {
            command: ARGS.join(" "),
            output: String::from_utf8_lossy(&output.stdout).into_owned(),
        })?;
        Ok(Some(name.strip_suffix('\n').unwrap_or(name).to_owned()))
    }

    // Temporary @/inv/convention-convergence exception: document history
    // converges here when 83c8632f lands.
    /// List commits touching a literal worktree-relative path, newest first,
    /// with git's default history simplification. One git process.
    ///
    /// # Errors
    ///
    /// Returns an error when git cannot start, fails, or reports malformed
    /// commit metadata.
    pub fn file_history(&self, path: &str) -> Result<Vec<FileHistoryCommit>, GitError> {
        let args = [
            "--literal-pathspecs",
            "log",
            "-z",
            "--no-show-signature",
            "--no-notes",
            "--encoding=UTF-8",
            "--format=%H%x00%an%x00%ct%x00%B",
            "--",
            path,
        ];
        let output = self.git.run(&self.worktree_root, args)?;
        let unexpected = |bytes: &[u8]| GitError::UnexpectedOutput {
            command: args.join(" "),
            output: String::from_utf8_lossy(bytes).into_owned(),
        };
        if !output.status.success() {
            return Err(unexpected(&output.stderr));
        }
        parse_file_history(&output.stdout).ok_or_else(|| unexpected(&output.stdout))
    }

    // Temporary @/inv/convention-convergence exception: snapshot migration
    // c140921c moves its origin lookup here.
    /// Read the `origin` remote URL with one git process.
    ///
    /// Returns `None` when the checkout has no `origin` remote.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::GitUnavailable`] if git cannot start, or
    /// [`GitError::UnexpectedOutput`] if git fails for another reason or its
    /// successful output is not one UTF-8 URL.
    pub fn origin_url(&self) -> Result<Option<String>, GitError> {
        const ARGS: [&str; 3] = ["remote", "get-url", "origin"];
        let output = self.git.run(&self.worktree_root, ARGS)?;
        if !output.status.success() {
            if output.status.code() == Some(2) {
                return Ok(None);
            }
            return Err(GitError::UnexpectedOutput {
                command: ARGS.join(" "),
                output: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let unexpected = || GitError::UnexpectedOutput {
            command: ARGS.join(" "),
            output: String::from_utf8_lossy(&output.stdout).into_owned(),
        };
        let stdout = std::str::from_utf8(&output.stdout).map_err(|_| unexpected())?;
        let url = stdout.strip_suffix('\n').ok_or_else(unexpected)?;
        Ok(Some(url.to_owned()))
    }

    // Temporary @/inv/convention-convergence exception: branch comparisons
    // converge here when fb5e8e8d (check_branch_divergence) and 47d62472
    // (validate_branch_drift) migrate their consumers.
    /// Compare this checkout's HEAD with a caller-selected base revision.
    ///
    /// # Errors
    ///
    /// Returns an error if git cannot start, produces a non-canonical OID, or
    /// fails to compare two resolved commits for a reason other than unrelated
    /// histories.
    pub fn compare_with_base(&self, base: &str) -> Result<BaseComparison, GitError> {
        let base_commit = match self.resolve_commit(base) {
            Ok(ArtifactVersion::Pinned(oid)) => oid,
            Ok(version) => {
                return Err(GitError::UnexpectedOutput {
                    command: format!("rev-parse --verify {base}^{{commit}}"),
                    output: version.as_str().to_owned(),
                });
            }
            Err(GitError::RevisionNotFound { .. }) => return Ok(BaseComparison::BaseUnresolved),
            Err(error) => return Err(error),
        };
        let args = ["merge-base", "HEAD", base_commit.as_str()];
        let output = self.git.run(&self.worktree_root, args)?;
        match output.status.code() {
            Some(0) => {
                let oid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                let merge_base = CanonicalCommitOid::parse(oid.clone()).map_err(|_| {
                    GitError::NonCanonicalOid {
                        revision: "HEAD".to_owned(),
                        output: oid,
                    }
                })?;
                Ok(BaseComparison::Compared {
                    merge_base,
                    base_commit,
                })
            }
            Some(1) => Ok(BaseComparison::NoMergeBase),
            _ => Err(GitError::UnexpectedOutput {
                command: args.join(" "),
                output: stderr(&output),
            }),
        }
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

/// A full lowercase SHA-1 or SHA-256 identity for any Git object.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitObjectId(String);

impl GitObjectId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn parse(value: &str) -> Option<Self> {
        (matches!(value.len(), 40 | 64)
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        .then(|| Self(value.to_owned()))
    }
}

/// Git's stored object kinds; tree entries such as symlinks remain blobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitObjectType {
    Blob,
    Tree,
    Commit,
    Tag,
}

impl GitObjectType {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "blob" => Some(Self::Blob),
            "tree" => Some(Self::Tree),
            "commit" => Some(Self::Commit),
            "tag" => Some(Self::Tag),
            _ => None,
        }
    }
}

/// An object and its exact uncompressed contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitObject {
    id: GitObjectId,
    object_type: GitObjectType,
    bytes: Vec<u8>,
}

impl GitObject {
    pub fn id(&self) -> &GitObjectId {
        &self.id
    }

    pub fn object_type(&self) -> GitObjectType {
        self.object_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// An incremental reader owning one Git child, reaped on drop or protocol failure.
pub struct GitObjectReader {
    process: CatFileProcess,
}

impl GitObjectReader {
    /// Read another name, including names discovered from an earlier answer.
    /// Missing names return `None`; newline or NUL input is rejected before sending.
    /// I/O or protocol errors end the session and return [`GitError::UnexpectedOutput`].
    pub fn read(&mut self, name: &str) -> Result<Option<GitObject>, GitError> {
        let Some(header) = self.process.request(name)? else {
            return Ok(None);
        };
        let contents = (|| {
            let output = self.process.output()?;
            let mut bytes = Vec::new();
            output.take(header.size).read_to_end(&mut bytes)?;
            let mut separator = [0];
            output.read_exact(&mut separator)?;
            if bytes.len() as u64 != header.size || separator[0] != b'\n' {
                return Err(invalid_object_output("invalid object body framing"));
            }
            Ok(bytes)
        })();
        let bytes = contents.map_err(|error| self.process.fail(error))?;
        Ok(Some(GitObject {
            id: header.id,
            object_type: header.object_type,
            bytes,
        }))
    }
}

impl GitCheckout {
    /// Start one `cat-file --batch` process for any number of incremental reads.
    /// Returns [`GitError::GitUnavailable`] when the executable cannot start.
    pub fn object_reader(&self) -> Result<GitObjectReader, GitError> {
        Ok(GitObjectReader {
            process: CatFileProcess::start(self, CatFileMode::Contents)?,
        })
    }

    /// Look up object kinds in input order through one `cat-file --batch-check`.
    /// Missing names return `None`. Every name is validated before starting Git.
    /// Startup, I/O and protocol failures use the module's typed errors.
    pub fn object_types(
        &self,
        names: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Vec<Option<GitObjectType>>, GitError> {
        let names = names
            .into_iter()
            .map(|name| {
                validate_object_name(name.as_ref())?;
                Ok(name.as_ref().to_owned())
            })
            .collect::<Result<Vec<_>, GitError>>()?;
        let mut process = CatFileProcess::start(self, CatFileMode::Types)?;
        let types = names
            .iter()
            .map(|name| {
                process
                    .request(name)
                    .map(|header| header.map(|header| header.object_type))
            })
            .collect::<Result<Vec<_>, _>>()?;
        process.finish()?;
        Ok(types)
    }
}

#[derive(Clone, Copy)]
enum CatFileMode {
    Contents,
    Types,
}

impl CatFileMode {
    fn argument(self) -> &'static str {
        match self {
            Self::Contents => "--batch",
            Self::Types => "--batch-check",
        }
    }
}

struct ObjectHeader {
    id: GitObjectId,
    object_type: GitObjectType,
    size: u64,
}

struct CatFileProcess {
    child: Child,
    stdout: Option<BufReader<ChildStdout>>,
    stderr_reader: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
    mode: CatFileMode,
    reaped: bool,
}

impl CatFileProcess {
    fn start(checkout: &GitCheckout, mode: CatFileMode) -> Result<Self, GitError> {
        let mut child = checkout
            .git
            .command(&checkout.worktree_root, ["cat-file", mode.argument()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| checkout.git.unavailable(source))?;
        let stdout = child.stdout.take().map(BufReader::new);
        let mut process = Self {
            child,
            stdout,
            stderr_reader: None,
            mode,
            reaped: false,
        };
        let setup = (|| {
            let mut stderr = process
                .child
                .stderr
                .take()
                .ok_or_else(|| io::Error::other("git did not expose stderr"))?;
            thread::Builder::new().spawn(move || {
                // Drain continuously; retain only bounded diagnostic text.
                let mut bytes = Vec::new();
                stderr.by_ref().take(64 * 1024).read_to_end(&mut bytes)?;
                io::copy(&mut stderr, &mut io::sink())?;
                Ok(bytes)
            })
        })();
        process.stderr_reader = Some(setup.map_err(|error| process.fail(error))?);
        Ok(process)
    }

    fn output(&mut self) -> io::Result<&mut BufReader<ChildStdout>> {
        self.stdout
            .as_mut()
            .ok_or_else(|| io::Error::other("git object session is closed"))
    }

    fn request(&mut self, name: &str) -> Result<Option<ObjectHeader>, GitError> {
        validate_object_name(name)?;
        let answer = (|| {
            let input = self
                .child
                .stdin
                .as_mut()
                .ok_or_else(|| io::Error::other("git object session is closed"))?;
            writeln!(input, "{name}")?;
            input.flush()?;
            let mut header = Vec::new();
            self.output()?.read_until(b'\n', &mut header)?;
            parse_object_header(name, &header)
        })();
        answer.map_err(|error| self.fail(error))
    }

    fn finish(&mut self) -> Result<(), GitError> {
        self.child.stdin.take();
        let finished = (|| {
            if !self.output()?.fill_buf()?.is_empty() {
                return Err(invalid_object_output("unexpected trailing response"));
            }
            let status = self.child.wait()?;
            self.reaped = true;
            if !status.success() {
                return Err(io::Error::other(format!("git exited with {status}")));
            }
            self.stderr().map(|_| ())
        })();
        finished.map_err(|error| self.fail(error))
    }

    fn stderr(&mut self) -> io::Result<Vec<u8>> {
        self.stderr_reader
            .take()
            .map(|reader| {
                reader
                    .join()
                    .map_err(|_| io::Error::other("git stderr reader panicked"))?
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }

    fn stop(&mut self) {
        self.child.stdin.take();
        self.stdout.take();
        if !self.reaped {
            let _ = self.child.kill();
            self.reaped = self.child.wait().is_ok();
        }
    }

    fn fail(&mut self, error: io::Error) -> GitError {
        self.stop();
        let diagnostic = match self.stderr() {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(error) => error.to_string(),
        };
        GitError::UnexpectedOutput {
            command: format!("cat-file {}", self.mode.argument()),
            output: format!("{error}: {diagnostic}"),
        }
    }
}

impl Drop for CatFileProcess {
    fn drop(&mut self) {
        self.stop();
        let _ = self.stderr();
    }
}

fn validate_object_name(name: &str) -> Result<(), GitError> {
    if name.contains(['\n', '\0']) {
        Err(GitError::InvalidPath(
            "git object names cannot contain a newline or NUL".into(),
        ))
    } else {
        Ok(())
    }
}

fn invalid_object_output(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn parse_object_header(name: &str, bytes: &[u8]) -> io::Result<Option<ObjectHeader>> {
    let malformed = || {
        invalid_object_output(format!(
            "invalid object header: {:?}",
            String::from_utf8_lossy(bytes)
        ))
    };
    let line = std::str::from_utf8(bytes)
        .ok()
        .and_then(|header| header.strip_suffix('\n'))
        .ok_or_else(malformed)?;
    if line.strip_suffix(" missing") == Some(name) {
        return Ok(None);
    }
    let mut fields = line.split(' ');
    let id = fields
        .next()
        .and_then(GitObjectId::parse)
        .ok_or_else(malformed)?;
    let object_type = fields
        .next()
        .and_then(GitObjectType::parse)
        .ok_or_else(malformed)?;
    let size = fields
        .next()
        .filter(|size| !size.is_empty() && size.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|size| size.parse().ok())
        .ok_or_else(malformed)?;
    if fields.next().is_some() {
        return Err(malformed());
    }
    Ok(Some(ObjectHeader {
        id,
        object_type,
        size,
    }))
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

/// A commit in a path's history; the message retains its whitespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHistoryCommit {
    commit: CanonicalCommitOid,
    author: String,
    commit_time: i64,
    message: String,
}

impl FileHistoryCommit {
    pub fn commit(&self) -> &CanonicalCommitOid {
        &self.commit
    }

    pub fn author(&self) -> &str {
        &self.author
    }

    /// Committer time as Unix seconds.
    pub fn commit_time(&self) -> i64 {
        self.commit_time
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

fn parse_file_history(output: &[u8]) -> Option<Vec<FileHistoryCommit>> {
    if output.is_empty() {
        return Some(Vec::new());
    }
    let stdout = std::str::from_utf8(output).ok()?.strip_suffix('\0')?;
    let fields: Vec<_> = stdout.split('\0').collect();
    let records = fields.chunks_exact(4);
    if !records.remainder().is_empty() {
        return None;
    }
    records
        .map(|record| {
            Some(FileHistoryCommit {
                commit: CanonicalCommitOid::parse(record[0]).ok()?,
                author: record[1].to_owned(),
                commit_time: record[2].parse().ok()?,
                message: record[3].to_owned(),
            })
        })
        .collect()
}

fn parse_head_output<T>(
    command: &[&str],
    output: &Output,
    parse: impl FnOnce(&str) -> Option<T>,
) -> Result<T, GitError> {
    let parsed = match (
        output.status.success(),
        output.stderr.is_empty(),
        std::str::from_utf8(&output.stdout),
    ) {
        (true, true, Ok(stdout)) => parse(stdout),
        _ => None,
    };
    parsed.ok_or_else(|| GitError::UnexpectedOutput {
        command: command.join(" "),
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ),
    })
}

fn parse_head_identity(stdout: &str) -> Option<Head> {
    let (oid, reference) = stdout.strip_suffix('\n')?.split_once('\n')?;
    let commit = CanonicalCommitOid::parse(oid).ok()?;
    let branch = match reference {
        "HEAD" => None,
        reference => Some(parse_head_branch(reference)?.to_owned()),
    };
    Some(Head { commit, branch })
}

fn parse_head_branch(reference: &str) -> Option<&str> {
    reference
        .strip_prefix("refs/heads/")
        .filter(|branch| !branch.is_empty() && !branch.bytes().any(|byte| byte.is_ascii_control()))
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

    // Temporary @/inv/convention-convergence exception: dcf88b6f moves
    // pinned inspection onto object_reader and removes this batch parser.
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
    fn test_parse_file_history_rejects_malformed_records() {
        let oid = "a".repeat(40);
        let record = format!("{oid}\0author\x00123\0message\n\0");
        let malformed = [
            record.trim_end_matches('\0').as_bytes().to_vec(),
            format!("{oid}\0author\x00123\0").into_bytes(),
            format!("{record}extra\0").into_bytes(),
            b"short-oid\0author\x00123\0message\0".to_vec(),
            format!("{oid}\0author\0invalid-time\0message\0").into_bytes(),
            format!("{oid}\0author\x009223372036854775808\0message\0").into_bytes(),
            [record.as_bytes(), &[0xff]].concat(),
        ];

        for output in malformed {
            assert!(parse_file_history(&output).is_none(), "{output:?}");
        }
    }

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
    fn test_parse_head_identity_rejects_malformed_protocol_records() {
        let oid = "1".repeat(40);
        [
            String::new(),
            format!("{oid}\n"),
            format!("{oid}\nrefs/heads/"),
            format!("{oid}\nrefs/heads/\n"),
            format!("{oid}\nrefs/tags/tag\n"),
            format!("{oid}\nHEAD\nextra\n"),
            format!("{oid}\nrefs/heads/branch\0suffix\n"),
            "invalid-object\nrefs/heads/branch\n".to_owned(),
        ]
        .iter()
        .for_each(|output| assert!(parse_head_identity(output).is_none(), "{output:?}"));
    }

    #[cfg(unix)]
    #[test]
    fn test_parse_head_output_rejects_failed_noisy_or_non_utf8_output() {
        use std::os::unix::process::ExitStatusExt;

        [
            (1, b"apparently valid".as_slice(), b"".as_slice()),
            (0, b"apparently valid".as_slice(), b"warning".as_slice()),
            (0, b"\xff".as_slice(), b"".as_slice()),
        ]
        .into_iter()
        .for_each(|(status, stdout, stderr)| {
            let output = Output {
                status: std::process::ExitStatus::from_raw(status << 8),
                stdout: stdout.to_vec(),
                stderr: stderr.to_vec(),
            };
            assert!(matches!(
                parse_head_output(&["head-test"], &output, |_| Some(())),
                Err(GitError::UnexpectedOutput { .. })
            ));
        });
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
