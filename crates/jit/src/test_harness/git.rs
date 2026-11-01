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
/// The real git is the first executable `git` under an absolute `PATH` entry
/// that is not itself a stand-in, canonicalized; relative entries are skipped.
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
    real_git: PathBuf,
}

/// The stand-in script's first lines, which identify a stand-in on `PATH`.
#[cfg(unix)]
const STAND_IN_HEADER: &str = "#!/bin/sh\n# jit test-harness git stand-in\n";

#[cfg(unix)]
impl GitStandIn {
    /// Create the stand-in, resolving the real git on the current `PATH`.
    pub fn new() -> Self {
        Self::resolving_on(&std::env::var_os("PATH").unwrap_or_default())
    }

    fn resolving_on(search_path: &std::ffi::OsStr) -> Self {
        let stand_in = Self {
            dir: tempfile::TempDir::new().unwrap(),
            real_git: resolve_real_git(search_path),
        };
        std::fs::create_dir(stand_in.bin_dir()).unwrap();
        std::fs::create_dir(stand_in.records_dir()).unwrap();
        let script = format!(
            r#"{STAND_IN_HEADER}records={records}
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
            git = shell_quote(&stand_in.real_git),
        );
        crate::test_utils::write_executable(&stand_in.program(), &script).unwrap();
        stand_in
    }

    /// The stand-in executable, named `git`.
    pub fn program(&self) -> PathBuf {
        self.bin_dir().join("git")
    }

    /// The absolute path of the real git the stand-in runs.
    pub fn real_git(&self) -> &Path {
        &self.real_git
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
}

#[cfg(unix)]
fn resolve_real_git(search_path: &std::ffi::OsStr) -> PathBuf {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    let is_stand_in = |path: &Path| {
        let mut header = vec![0; STAND_IN_HEADER.len()];
        std::fs::File::open(path)
            .and_then(|mut file| file.read_exact(&mut header))
            .is_ok_and(|()| header == STAND_IN_HEADER.as_bytes())
    };
    std::env::split_paths(search_path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("git"))
        .filter(|candidate| {
            std::fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
        .filter_map(|candidate| std::fs::canonicalize(candidate).ok())
        .find(|candidate| !is_stand_in(candidate))
        .expect("a git executable under an absolute PATH entry")
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
