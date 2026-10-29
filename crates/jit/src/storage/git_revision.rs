//! Git-backed canonical commit resolution and pinned artifact reads.
//!
//! Revision resolution deliberately shells out to Git with the exact
//! `rev-parse --verify <rev>^{commit}` contract. This preserves Git's own
//! symbolic and abbreviated revision semantics while publishing only a full,
//! hash-algorithm-agnostic OID to the domain layer.

use crate::domain::artifact_plan::ArtifactVersion;
use crate::repository_state::RepositoryTargetKind;
use crate::storage::validate_repo_relative_path;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::thread;
use thiserror::Error;

/// Maximum pinned paths passed to one `git ls-tree` process, keeping the
/// argument list well inside the platform limit.
const LS_TREE_PATHS_PER_PROCESS: usize = 1024;

/// Errors from canonical revision resolution and historical blob reads.
#[derive(Debug, Clone, Error)]
pub enum GitRevisionError {
    /// The configured Git executable could not be started.
    #[error("git is unavailable at {program}: {source}")]
    GitUnavailable {
        program: PathBuf,
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

/// Reusable Git boundary for version-aware artifact planning.
#[derive(Debug, Clone)]
pub struct GitRevisionResolver {
    repo_root: PathBuf,
    git_program: PathBuf,
}

impl GitRevisionResolver {
    /// Resolve revisions relative to `repo_root` using the `git` found on PATH.
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self::with_git_program(repo_root, "git")
    }

    /// Resolve revisions with an explicit Git executable.
    ///
    /// This supports hermetic embedders and makes the unavailable-Git failure
    /// contract directly testable without modifying process-global PATH.
    pub fn with_git_program(
        repo_root: impl Into<PathBuf>,
        git_program: impl Into<PathBuf>,
    ) -> Self {
        Self {
            repo_root: repo_root.into(),
            git_program: git_program.into(),
        }
    }

    /// Canonicalize `revision` with exact
    /// `git rev-parse --verify <revision>^{commit}` semantics.
    ///
    /// Both full SHA-1 and SHA-256 OIDs are accepted; abbreviated and symbolic
    /// inputs are returned only as Git's full canonical lowercase commit OID.
    pub fn resolve_commit(&self, revision: &str) -> Result<ArtifactVersion, GitRevisionError> {
        let peeled = format!("{revision}^{{commit}}");
        let output = self.run(["rev-parse", "--verify", peeled.as_str()])?;
        if !output.status.success() {
            return Err(GitRevisionError::RevisionNotFound {
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
    ) -> Result<PinnedArtifactRead, GitRevisionError> {
        validate_path(path)?;
        let version = self.resolve_commit(revision)?;
        let object = format!("{}:{path}", version.as_str());
        let oid_output = self.run(["rev-parse", "--verify", object.as_str()])?;
        if !oid_output.status.success() {
            return Err(GitRevisionError::PinnedReadFailed {
                revision: revision.to_string(),
                path: path.to_string(),
                stderr: stderr(&oid_output),
            });
        }
        let blob_oid = String::from_utf8_lossy(&oid_output.stdout)
            .trim()
            .to_string();
        let output = self.run(["cat-file", "blob", object.as_str()])?;
        if !output.status.success() {
            return Err(GitRevisionError::PinnedReadFailed {
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
    ) -> BTreeMap<(String, String), Result<PinnedTargetRead, GitRevisionError>> {
        if requests.is_empty() {
            return BTreeMap::new();
        }
        let previous_cache = std::mem::take(cache);
        let mut next_cache = BTreeMap::new();
        let mut results: BTreeMap<(String, String), Result<PinnedTargetRead, GitRevisionError>> =
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
                        Err(GitRevisionError::RevisionNotFound {
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
                            Err(GitRevisionError::PinnedReadFailed {
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
                                        Err(GitRevisionError::PinnedReadFailed {
                                            revision,
                                            path,
                                            stderr: reason.clone(),
                                        }),
                                    );
                                }
                                None => {
                                    results.insert(
                                        (revision.clone(), path.clone()),
                                        Err(GitRevisionError::PinnedReadFailed {
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
    ) -> Result<Result<BTreeMap<Vec<u8>, TreeEntry>, String>, GitRevisionError> {
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
                let output = self.run(arguments)?;
                if !output.status.success() {
                    return Ok(Err(stderr(&output)));
                }
                Ok(parse_pinned_tree_entries(&output.stdout).map(|entries| {
                    listed.extend(entries);
                    listed
                }))
            })
    }

    fn run<'a>(
        &self,
        arguments: impl IntoIterator<Item = &'a str>,
    ) -> Result<Output, GitRevisionError> {
        Command::new(&self.git_program)
            .current_dir(&self.repo_root)
            .args(arguments)
            .output()
            .map_err(|source| GitRevisionError::GitUnavailable {
                program: self.git_program.clone(),
                source: Arc::new(source),
            })
    }

    fn resolve_commits(
        &self,
        revisions: &BTreeSet<String>,
    ) -> BTreeMap<String, Result<ArtifactVersion, GitRevisionError>> {
        if revisions.is_empty() {
            return BTreeMap::new();
        }
        let peeled = revisions
            .iter()
            .map(|revision| format!("{revision}^{{commit}}"))
            .collect::<Vec<_>>();
        let mut arguments = vec!["rev-parse", "--end-of-options"];
        arguments.extend(peeled.iter().map(String::as_str));
        let output = match self.run(arguments) {
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
    ) -> BTreeMap<String, Result<ArtifactVersion, GitRevisionError>> {
        revisions
            .iter()
            .map(|revision| (revision.clone(), self.resolve_commit(revision)))
            .collect()
    }

    fn read_blob_batch(
        &self,
        oids: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, Result<Vec<u8>, String>>, GitRevisionError> {
        let mut child = Command::new(&self.git_program)
            .current_dir(&self.repo_root)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| GitRevisionError::GitUnavailable {
                program: self.git_program.clone(),
                source: Arc::new(source),
            })?;
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
            .map_err(|error| GitRevisionError::PinnedReadFailed {
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
        let status = wait_result.map_err(|error| GitRevisionError::PinnedReadFailed {
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
        parse_cat_file_batch(&bytes, oids).map_err(|reason| GitRevisionError::PinnedReadFailed {
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

fn validate_path(path: &str) -> Result<(), GitRevisionError> {
    validate_repo_relative_path(path)
        .map_err(|error| GitRevisionError::InvalidPath(error.to_string()))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn canonical_version(revision: &str, output: &str) -> Result<ArtifactVersion, GitRevisionError> {
    let oid = output.trim().to_string();
    ArtifactVersion::pinned(oid.clone()).map_err(|_| GitRevisionError::NonCanonicalOid {
        revision: revision.to_string(),
        output: oid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            Err(GitRevisionError::NonCanonicalOid { .. })
        ));
        assert!(matches!(
            canonical_version("HEAD", "0123456789ABCDEF0123456789ABCDEF01234567"),
            Err(GitRevisionError::NonCanonicalOid { .. })
        ));
    }
}
