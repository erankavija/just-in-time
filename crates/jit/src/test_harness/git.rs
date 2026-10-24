//! Git fixtures for tests: a repository built through the git CLI and, on
//! Unix, a counting stand-in for the git executable.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A git repository built through the git CLI, with a repository-local
/// identity so commits never depend on the host's configuration.
pub struct GitRepository {
    root: PathBuf,
}

impl GitRepository {
    /// Initialize a repository at `root`, creating the directory if needed.
    pub fn init(root: impl Into<PathBuf>) -> Self {
        let repository = Self { root: root.into() };
        std::fs::create_dir_all(&repository.root).unwrap();
        repository.git(&["init", "--quiet"]);
        repository.git(&["config", "user.name", "Test User"]);
        repository.git(&["config", "user.email", "test@example.com"]);
        repository.git(&["config", "commit.gpgsign", "false"]);
        repository
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write `contents` to the worktree-relative `path`, creating parents.
    pub fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> &Self {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
        self
    }

    /// Stage every worktree change, commit it, and return the new commit's id.
    pub fn commit(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "--message", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Run git in the repository, panicking on failure, and return its
    /// trimmed standard output.
    pub fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }
}

/// A stand-in for the git executable that records each invocation, then runs
/// the real git found on `PATH` when the stand-in was created.
///
/// The stand-in is a file named `git` alone in its directory, so a test can
/// pass [`program`](Self::program) as an explicit program or put the
/// directory first on a child's `PATH` through
/// [`search_path`](Self::search_path). Each invocation publishes one record
/// under a sequence number claimed by an exclusive hard link, so concurrent
/// processes never interleave records and arguments keep their exact bytes.
#[cfg(unix)]
pub struct GitStandIn {
    dir: tempfile::TempDir,
}

#[cfg(unix)]
impl GitStandIn {
    /// Create the stand-in, resolving the real git on the current `PATH`.
    pub fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;

        let real_git = Self::resolve_real_git();
        let dir = tempfile::TempDir::new().unwrap();
        let stand_in = Self { dir };
        std::fs::create_dir(stand_in.bin_dir()).unwrap();
        std::fs::create_dir(stand_in.records_dir()).unwrap();
        let script = format!(
            r#"#!/bin/sh
records={records}
staged="$records/.$$"
if [ "$#" -gt 0 ]; then printf '%s\0' "$@"; fi > "$staged" || exit 125
count() {{ echo "$#"; }}
sequence=$(count "$records"/*)
until ln "$staged" "$records/$sequence" 2>/dev/null; do
  [ -e "$records/$sequence" ] || exit 125
  sequence=$((sequence + 1))
done
rm -f "$staged"
exec {git} "$@"
"#,
            records = shell_quote(&stand_in.records_dir()),
            git = shell_quote(&real_git),
        );
        let program = stand_in.program();
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        stand_in
    }

    /// The stand-in executable, named `git`.
    pub fn program(&self) -> PathBuf {
        self.bin_dir().join("git")
    }

    /// The directory holding only the stand-in.
    pub fn bin_dir(&self) -> PathBuf {
        self.dir.path().join("bin")
    }

    /// A `PATH` value with the stand-in's directory before the current `PATH`.
    pub fn search_path(&self) -> std::ffi::OsString {
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        std::env::join_paths(
            std::iter::once(self.bin_dir()).chain(std::env::split_paths(&inherited)),
        )
        .unwrap()
    }

    /// The invocations recorded so far, in the order they claimed a record.
    ///
    /// Read once the processes of interest have exited; a running process may
    /// not have published its record yet.
    pub fn invocations(&self) -> Vec<GitInvocation> {
        let mut records: Vec<(u64, PathBuf)> = std::fs::read_dir(self.records_dir())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter_map(|path| {
                let sequence = path.file_name()?.to_str()?.parse().ok()?;
                Some((sequence, path))
            })
            .collect();
        records.sort();
        records
            .into_iter()
            .map(|(_, path)| GitInvocation::from_record(&std::fs::read(path).unwrap()))
            .collect()
    }

    fn records_dir(&self) -> PathBuf {
        self.dir.path().join("invocations")
    }

    fn resolve_real_git() -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&path)
            .map(|dir| dir.join("git"))
            .find(|candidate| {
                std::fs::metadata(candidate)
                    .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            })
            .expect("a git executable on PATH")
    }
}

#[cfg(unix)]
impl Default for GitStandIn {
    fn default() -> Self {
        Self::new()
    }
}

/// One recorded git process.
#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInvocation {
    directory: Option<PathBuf>,
    args: Vec<String>,
}

#[cfg(unix)]
impl GitInvocation {
    /// The directory of a leading `-C <dir>` pair, if the invocation had one.
    pub fn directory(&self) -> Option<&Path> {
        self.directory.as_deref()
    }

    /// The arguments after any leading `-C <dir>` pair, which the git module
    /// puts before every invocation.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Parse a record of NUL-terminated arguments.
    fn from_record(record: &[u8]) -> Self {
        let all: Vec<String> = record
            .strip_suffix(b"\0")
            .map(|body| {
                body.split(|byte| *byte == 0)
                    .map(|arg| String::from_utf8(arg.to_vec()).unwrap())
                    .collect()
            })
            .unwrap_or_default();
        match all.as_slice() {
            [flag, directory, rest @ ..] if flag == "-C" => Self {
                directory: Some(PathBuf::from(directory)),
                args: rest.to_vec(),
            },
            _ => Self {
                directory: None,
                args: all,
            },
        }
    }
}

#[cfg(unix)]
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', r"'\''"))
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
