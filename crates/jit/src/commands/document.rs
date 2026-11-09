//! Document reference operations

use super::*;
use crate::domain::list_envelope::{collection_keys, ListEnvelope};
use schemars::JsonSchema;
use serde::Serialize;

collection_keys! {
    /// Collection key of [`ArtifactConformanceResponse`].
    ArtifactsKey = "artifacts" as "ArtifactConformanceResponse";
}

/// Response for `doc dir` command.
///
/// One resolved value rather than a collection, so the shape is a flat named
/// object: the issue whose directory this is, the area the caller named, and
/// the repository-relative directory
/// ([`resolve_artifact_directory`](crate::domain::artifact_directory::resolve_artifact_directory)
/// derives the name inside the area). `area` echoes the caller's spelling
/// rather than a normalized one, so a response is readable against the request
/// that produced it, while `directory` is normalized.
///
/// The directory is a name: it is reported whether or not anything has been
/// written there, and producing this response creates nothing.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ArtifactDirectoryResponse {
    /// Full identifier of the issue the directory belongs to.
    pub issue_id: IssueId,
    /// Short identifier the directory name is built from.
    pub short_id: String,
    /// Issue-scoped area the caller named.
    pub area: String,
    /// Repository-relative directory the issue owns in that area.
    pub directory: String,
}

/// Response for `doc conformance`, the advisory artifact-location report.
///
/// `count` is the length of `artifacts`, which are ordered by area and then by
/// path. `areas` states the registry that was walked, which is exactly
/// [`DocumentationConfig::issue_scoped_areas`](crate::config::DocumentationConfig::issue_scoped_areas),
/// so a machine caller reads the scanned set instead of assuming one.
///
/// The report is advice (`@/issue/8e071e18/decision/D-7`): an entry states
/// where an artifact sits and where its owner's directory is, and blocks
/// nothing.
pub type ArtifactConformanceResponse =
    ListEnvelope<ArtifactsKey, ArtifactConformanceEntry, ConformanceScope>;

/// Fields of [`ArtifactConformanceResponse`] beside its `artifacts`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ConformanceScope {
    /// Declared issue-scoped areas the report walked.
    pub areas: Vec<String>,
}

/// One artifact `doc conformance` names.
///
/// `status` is `nonconforming` when the owning issue is known and the artifact
/// sits outside the directory that issue owns, and `unattributed` when the
/// report's inputs settle no single owner. `issue_id` and
/// `canonical_directory` are carried by a `nonconforming` entry and absent from
/// an `unattributed` one, which claims no owner and no destination.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ArtifactConformanceEntry {
    /// Repository-relative path of the artifact, which is a directory when a
    /// whole directory sits outside its owner's.
    pub path: String,
    /// Declared issue-scoped area the artifact was found under.
    pub area: String,
    /// Short identifier the artifact's own name opens with, empty when the name
    /// carries none and a document reference resolved the owner instead.
    pub short_id: String,
    /// `nonconforming` or `unattributed`.
    pub status: String,
    /// Full identifier of the owning issue, for a `nonconforming` entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<IssueId>,
    /// Repository-relative directory the owner owns in that area, for a
    /// `nonconforming` entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_directory: Option<String>,
}

impl From<crate::domain::artifact_conformance::ReportedArtifact> for ArtifactConformanceEntry {
    fn from(artifact: crate::domain::artifact_conformance::ReportedArtifact) -> Self {
        use crate::domain::artifact_conformance::ArtifactDisposition;

        let status = artifact.disposition.as_str().to_string();
        let (issue_id, canonical_directory) = match artifact.disposition {
            ArtifactDisposition::Nonconforming {
                issue_id,
                canonical_directory,
            } => (Some(issue_id), Some(canonical_directory)),
            ArtifactDisposition::Unattributed => (None, None),
        };
        Self {
            path: artifact.path,
            area: artifact.area,
            short_id: artifact.short_id,
            status,
            issue_id,
            canonical_directory,
        }
    }
}

/// An issue document's bytes at its selected version.
#[derive(Debug)]
pub struct DocumentReading {
    pub document: crate::domain::DocumentReference,
    pub bytes: Vec<u8>,
    /// Storage's resolved short commit, or `working-tree`.
    pub commit: String,
}

impl DocumentReading {
    /// The bytes decoded as UTF-8, with invalid sequences replaced by U+FFFD.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

impl From<DocumentReading> for DocumentContentResult {
    fn from(reading: DocumentReading) -> Self {
        let content = reading.text();
        Self {
            path: reading.document.path,
            label: reading.document.label,
            commit: reading.commit,
            doc_type: reading.document.doc_type,
            content,
        }
    }
}

const EXTERNAL_URL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const EXTERNAL_URL_WORKERS: usize = 4;

/// Bounds for one document reference edit, which mutates one named issue while
/// also reading the document and its assets: the shared issue-mutation bounds
/// at the depth those worktree paths reach.
fn document_capture_budget() -> crate::repository_state::CaptureBudget {
    crate::repository_state::CaptureBudget {
        max_depth: 8,
        ..super::issue_mutation_capture_budget()
    }
}

struct DerivedDocumentMutation<T> {
    outcome: T,
    intents: Vec<crate::repository_state::MutationIntent>,
}

struct CapturedDocumentScan {
    format: Option<String>,
    assets: Vec<crate::document::Asset>,
    required_paths: std::collections::BTreeSet<crate::repository_state::VirtualPath>,
    warning: Option<DocumentScanWarning>,
}

enum DocumentScanWarning {
    Missing(String),
    Failed(String),
}

enum DocumentScanSource {
    Worktree(crate::repository_state::VirtualPath),
    Pinned { revision: String, path: String },
}

/// Capture bounds for one link check: each pinned reference's content, plus the
/// kinds of every reference and of the assets and link targets a pinned
/// reference names at its commit.
const LINK_CHECK_CAPTURE_BUDGET: crate::repository_state::CaptureBudget =
    crate::repository_state::CaptureBudget {
        max_listings: 0,
        max_bytes: super::CAPTURE_MAX_BYTES,
        max_depth: 32,
    };

/// One document reference as the link checker read it at the version it names.
enum CheckedDocument {
    /// The reference's file does not exist at that version.
    Unresolved(crate::document::UnresolvedDocumentReference),
    /// An unpinned reference, whose neighbourhood the working tree answers for.
    WorkingTree,
    /// A commit-pinned reference, read at its commit.
    Pinned(PinnedDocumentReading),
}

/// A pinned document's content and neighbourhood, read at its commit.
struct PinnedDocumentReading {
    links: Vec<crate::document::InternalLink>,
    target_kinds:
        std::collections::BTreeMap<std::path::PathBuf, crate::document::DocumentTargetKind>,
    scan_error: Option<String>,
}

impl PinnedDocumentReading {
    /// Captured kind of `path` at the pinned commit.
    ///
    /// Answers only for the assets and link targets the capture requested; any
    /// other path is reported absent rather than read from the working tree.
    fn target_kind(&self, path: &std::path::Path) -> crate::document::DocumentTargetKind {
        self.target_kinds
            .get(path)
            .copied()
            .unwrap_or(crate::document::DocumentTargetKind::Missing)
    }

    /// The internal links the pinned content names, or why it could not be
    /// scanned.
    fn links(&self) -> std::result::Result<Vec<crate::document::InternalLink>, String> {
        match &self.scan_error {
            Some(message) => Err(message.clone()),
            None => Ok(self.links.clone()),
        }
    }
}

impl DocumentScanWarning {
    fn message(&self) -> &str {
        match self {
            Self::Missing(message) | Self::Failed(message) => message,
        }
    }
}

fn render_document_diff(
    checkout: &crate::storage::git::GitCheckout,
    checkout_top_level: &std::path::Path,
    repository_root: &std::path::Path,
    path: &str,
    from: &str,
    to: &str,
) -> Result<String> {
    use similar::{ChangeTag, TextDiff};

    let relative = crate::domain::relative_path::RootRelativePath::parse(path)
        .map_err(|error| crate::storage::PathReadError::InvalidPath(error.to_string()))?;
    if relative.is_root() {
        return Err(crate::storage::PathReadError::InvalidPath(
            "repository root is not a document path".to_string(),
        )
        .into());
    }
    let relative_root = repository_root
        .strip_prefix(checkout_top_level)
        .unwrap_or(std::path::Path::new(""));
    let relative_path = relative_root.join(relative.as_path());
    let relative_path = relative_path
        .to_str()
        .ok_or_else(|| anyhow!("document path is not valid UTF-8"))?;

    let mut reader = checkout.object_reader()?;
    let from_content = read_document_diff_version(&mut reader, relative_path, from)?;
    let to_content = read_document_diff_version(&mut reader, relative_path, to)?;

    let mut diff_output = format!("diff --git a/{path} b/{path}\n");
    diff_output.push_str(&format!("--- a/{path} ({from})\n"));
    diff_output.push_str(&format!("+++ b/{path} ({to})\n\n"));

    let diff = TextDiff::from_lines(&from_content, &to_content);
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        diff_output.push_str(&format!("{sign}{change}"));
    }

    Ok(diff_output)
}

fn read_document_diff_version(
    reader: &mut crate::storage::git::GitObjectReader,
    path: &str,
    revision: &str,
) -> Result<String> {
    use crate::{
        domain::artifact_plan::ArtifactVersion,
        storage::git::{GitError, GitObjectType},
    };

    let version = reader.resolve_commit(revision)?.into_version(revision)?;
    let ArtifactVersion::Pinned(commit) = version else {
        return Err(anyhow!(
            "internal error: commit resolution returned a working tree version"
        ));
    };
    let object_name = format!("{}:{path}", commit.as_str());
    let object = reader
        .read(&object_name)?
        .ok_or_else(|| GitError::PinnedReadFailed {
            revision: revision.to_owned(),
            path: path.to_owned(),
            stderr: "path does not identify an object".to_owned(),
        })?;
    if object.object_type() != GitObjectType::Blob {
        return Err(GitError::PinnedReadFailed {
            revision: revision.to_owned(),
            path: path.to_owned(),
            stderr: "path does not identify a blob".to_owned(),
        }
        .into());
    }

    std::str::from_utf8(object.bytes())
        .map(str::to_owned)
        .map_err(|error| anyhow!("document '{path}' is not UTF-8: {error}"))
}

/// Document operations of a [`CommandExecutor`].
pub struct DocumentOps<'a, S: IssueStore> {
    exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The document family.
    pub fn documents(&self) -> DocumentOps<'_, S> {
        DocumentOps { exec: self }
    }
}

impl<S: IssueStore> DocumentOps<'_, S> {
    #[allow(clippy::too_many_arguments)] // CLI command parameters - refactoring would reduce clarity
    pub fn add_document_reference(
        &self,
        issue_id: &IssueId,
        path: &str,
        commit: Option<&str>,
        label: Option<&str>,
        doc_type: Option<&str>,
        skip_scan: bool,
    ) -> Result<(DocumentAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = issue_id.clone();
        self.publish_document_add(&full_id, path, commit, label, doc_type, skip_scan)
    }

    /// The no-follow listing of the tree rooted at `root` at `scope`; a root
    /// that is absent, a file, or a symlink lists nothing.
    fn list_artifact_tree(
        &self,
        root: &str,
        scope: crate::domain::artifact_discovery::ArtifactListingScope,
    ) -> Result<Vec<String>> {
        use crate::domain::artifact_discovery::ArtifactEvidence;

        Ok(
            match crate::storage::artifact_inspection::inspect_artifact_evidence(
                &self.exec.storage,
                root,
                scope,
            )? {
                ArtifactEvidence::Directory { entries, .. } => entries,
                _ => Vec::new(),
            },
        )
    }

    /// Each declared issue-scoped area paired with its no-follow recursive
    /// listing at `scope`.
    ///
    /// The registry is read through the accessor that decides area membership
    /// everywhere else: a declared entry that normalizes to nothing names no
    /// area, so it is neither walked nor named. Each surviving area is named in
    /// its normalized spelling.
    fn list_issue_scoped_areas(
        &self,
        documentation: &crate::config::DocumentationConfig,
        scope: crate::domain::artifact_discovery::ArtifactListingScope,
    ) -> Result<Vec<(String, Vec<String>)>> {
        documentation
            .issue_scoped_areas()
            .into_iter()
            .filter(|area| documentation.is_issue_scoped_area(area))
            .map(|area| {
                let area = crate::domain::artifact_plan::normalize_artifact_path(&area);
                let entries = self.list_artifact_tree(&area, scope)?;
                Ok((area, entries))
            })
            .collect()
    }

    /// Each of `issues` paired with its documents: its explicit links followed
    /// by the documents it owns by location
    /// ([`derive_issue_documents`](crate::domain::artifact_directory::derive_issue_documents)).
    ///
    /// One issue lists only the directories it owns; several share one listing
    /// of the declared areas. A repository declaring none reads no tree.
    fn with_issue_documents(
        &self,
        issues: Vec<crate::domain::Issue>,
    ) -> Result<Vec<(crate::domain::Issue, Vec<crate::domain::DocumentReference>)>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::domain::artifact_discovery::ArtifactListingScope;

        let declarations = self.exec.captured_declarations()?;
        let declarations = DocumentDeclarations {
            documentation: declarations
                .config()
                .documentation
                .clone()
                .unwrap_or_default(),
            hierarchy: declarations.hierarchy()?,
        };
        let files = match issues.as_slice() {
            [issue] => crate::domain::artifact_directory::owned_artifact_directories(
                issue,
                &declarations.documentation,
                &declarations.hierarchy,
            )
            .iter()
            .map(|directory| {
                self.list_artifact_tree(directory, ArtifactListingScope::RecursiveFiles)
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
            _ => self
                .list_issue_scoped_areas(
                    &declarations.documentation,
                    ArtifactListingScope::RecursiveFiles,
                )?
                .into_iter()
                .flat_map(|(_, files)| files)
                .collect::<Vec<_>>(),
        };
        Ok(issues
            .into_iter()
            .map(|issue| {
                let documents = declarations.issue_documents(&issue, &files);
                (issue, documents)
            })
            .collect())
    }

    /// `issues` with `documents` replaced by the set every reader reports:
    /// explicit links, then the files each owns by location. For display only;
    /// nothing derived is stored.
    pub fn with_reported_documents(
        &self,
        issues: Vec<crate::domain::Issue>,
    ) -> Result<Vec<crate::domain::Issue>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        Ok(self
            .with_issue_documents(issues)?
            .into_iter()
            .map(|(issue, documents)| crate::domain::Issue { documents, ..issue })
            .collect())
    }

    /// `issue`'s documents as every reader reports them
    /// ([`with_reported_documents`](Self::with_reported_documents)).
    pub fn issue_documents(
        &self,
        issue: &crate::domain::Issue,
    ) -> Result<Vec<crate::domain::DocumentReference>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        Ok(self
            .with_reported_documents(vec![issue.clone()])?
            .into_iter()
            .flat_map(|issue| issue.documents)
            .collect())
    }

    /// The document of `issue` at `path`, explicit or derived.
    ///
    /// # Errors
    ///
    /// [`PathReadError::NotFound`](crate::storage::PathReadError::NotFound)
    /// when the issue has no document at `path`, and
    /// [`PathReadError::Other`](crate::storage::PathReadError::Other) when the
    /// declared areas cannot be listed.
    fn find_issue_document(
        &self,
        issue: crate::domain::Issue,
        path: &str,
    ) -> Result<crate::domain::DocumentReference, crate::storage::PathReadError>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::storage::PathReadError;

        let issue_id = issue.id.clone();
        self.with_issue_documents(vec![issue])
            .map_err(PathReadError::Other)?
            .into_iter()
            .flat_map(|(_, documents)| documents)
            .find(|document| document.path == path)
            .ok_or_else(|| {
                PathReadError::NotFound(format!(
                    "Document reference {path} not found in issue {issue_id}"
                ))
            })
    }

    /// List `issue_id`'s documents: its explicit links, then the files it owns
    /// by location under its artifact directories.
    pub fn list_document_references(
        &self,
        issue_id: &IssueId,
    ) -> Result<crate::commands::DocumentListResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = issue_id.clone();
        let issue = self.exec.storage.load_issue(&full_id)?;
        let documents = self.issue_documents(&issue)?;

        Ok(crate::commands::DocumentListResult::with_extras(
            documents,
            crate::commands::DocumentListScope { issue_id: full_id },
        ))
    }

    /// Resolve the canonical artifact directory `issue_id` owns in `area`.
    ///
    /// The caller names the area; the directory inside it is derived by
    /// [`resolve_artifact_directory`](crate::domain::artifact_directory::resolve_artifact_directory)
    /// from the issue's labels and the repository's configured type-to-namespace
    /// mapping, so no caller composes the name itself. The answer is a name:
    /// nothing is read from or written to the directory, and an issue whose
    /// artifacts still sit flat in the area resolves the same directory.
    ///
    /// # Errors
    ///
    /// An [`InvalidArgumentError`](crate::errors::InvalidArgumentError)
    /// carrying [`ArtifactDirectoryError`](crate::domain::artifact_directory::ArtifactDirectoryError)'s
    /// message when `area` is absent from the configured issue-scoped registry,
    /// so an undeclared area is an argument failure rather than a path. Also
    /// errors when `issue_id` resolves to no issue, when the issue cannot be
    /// loaded, or when the repository's configuration cannot be read.
    pub fn resolve_issue_artifact_directory(
        &self,
        issue_id: &IssueId,
        area: &str,
    ) -> Result<crate::commands::ArtifactDirectoryResponse>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = issue_id.clone();
        let issue = self.exec.storage.load_issue(&full_id)?;
        let declarations = self.exec.captured_declarations()?;
        let documentation = declarations
            .config()
            .documentation
            .clone()
            .unwrap_or_default();
        let hierarchy = declarations.hierarchy()?;

        let directory = crate::domain::artifact_directory::resolve_artifact_directory(
            &issue,
            area,
            &documentation,
            &hierarchy,
        )
        .map_err(|error| crate::errors::InvalidArgumentError::new(error.to_string()))?;

        Ok(crate::commands::ArtifactDirectoryResponse {
            short_id: issue.short_id(),
            issue_id: issue.id,
            area: area.to_string(),
            directory,
        })
    }

    /// Report the artifacts under the declared issue-scoped areas whose
    /// location disagrees with their owning issue's canonical directory.
    ///
    /// The walked registry is
    /// [`DocumentationConfig::issue_scoped_areas`](crate::config::DocumentationConfig::issue_scoped_areas)
    /// and nothing else, so a repository that declares no issue-scoped area is
    /// reported over no areas at all. Each area is listed with the archival
    /// planner's no-follow recursive walk at
    /// [`ArtifactListingScope::RecursiveEntries`](crate::domain::artifact_discovery::ArtifactListingScope::RecursiveEntries),
    /// which names the directories it descends through: a misplaced directory
    /// is an artifact whether or not it holds a file, and one holding nothing
    /// but further directories is reported the same way. The verdicts come from
    /// [`report_area_artifacts`](crate::domain::artifact_conformance::report_area_artifacts),
    /// which names the topmost offending component, so naming a directory and
    /// the files beneath it still reports that directory once.
    ///
    /// Read-only and advisory (`@/issue/8e071e18/decision/D-7`): the findings
    /// are the return value rather than a status a caller could gate on, and
    /// the run publishes, rewrites, and unlinks no repository content. The
    /// issue set the report resolves owners against — both the short ids those
    /// issues answer to and the document references they hold — is read through
    /// [`IssueStore::list_issues`]. A
    /// declared area that does not exist yet contributes no findings.
    ///
    /// # Errors
    ///
    /// An unreadable declared area, which is a failure of the walk rather than
    /// a finding, and the usual configuration and storage read failures.
    pub fn report_artifact_conformance(
        &self,
    ) -> Result<crate::commands::ArtifactConformanceResponse>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::domain::artifact_discovery::ArtifactListingScope;

        let declarations = self.exec.captured_declarations()?;
        let documentation = declarations
            .config()
            .documentation
            .clone()
            .unwrap_or_default();
        let hierarchy = declarations.hierarchy()?;
        let issues = self.exec.storage.list_issues()?;
        let listings =
            self.list_issue_scoped_areas(&documentation, ArtifactListingScope::RecursiveEntries)?;

        let artifacts = listings
            .iter()
            .map(|(area, paths)| {
                crate::domain::artifact_conformance::report_area_artifacts(
                    area,
                    paths,
                    &issues,
                    &documentation,
                    &hierarchy,
                )
                .map_err(anyhow::Error::from)
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .map(crate::commands::ArtifactConformanceEntry::from)
            .collect::<Vec<_>>();

        Ok(crate::commands::ArtifactConformanceResponse::with_extras(
            artifacts,
            crate::commands::ConformanceScope {
                areas: listings.into_iter().map(|(area, _)| area).collect(),
            },
        ))
    }

    pub fn remove_document_reference(
        &self,
        issue_id: &IssueId,
        path: &str,
    ) -> Result<(DocumentRemoveResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = issue_id.clone();
        self.publish_document_remove(&full_id, path)
    }

    /// Add or refresh one document reference from a coherent repository image.
    /// Asset discovery may expand the capture once; a changed document that
    /// reveals additional assets restarts from a fresh session.
    #[allow(clippy::too_many_arguments)]
    fn publish_document_add(
        &self,
        issue_id: &IssueId,
        path: &str,
        commit: Option<&str>,
        label: Option<&str>,
        doc_type: Option<&str>,
        skip_scan: bool,
    ) -> Result<(DocumentAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, RepositoryRootClass, VirtualPath};
        use std::collections::BTreeSet;

        let relative = crate::domain::relative_path::RootRelativePath::parse(path)
            .map_err(|error| crate::storage::PathReadError::InvalidPath(error.to_string()))?;
        if relative.is_root() {
            return Err(crate::storage::PathReadError::InvalidPath(
                "repository root is not a document path".to_string(),
            )
            .into());
        }
        let layout = self.exec.require_layout()?;
        let claims_guard = super::claims_mutation_guard(self.exec.require_worktree_paths()?)?;
        let issue_path = VirtualPath::issue_record(issue_id);
        let document_path = VirtualPath::from_root(RepositoryRootClass::Worktree, relative)
            .map_err(|error| crate::storage::PathReadError::InvalidPath(error.to_string()))?;
        let source = match commit {
            Some(revision) => DocumentScanSource::Pinned {
                revision: revision.to_string(),
                path: path.to_string(),
            },
            None => DocumentScanSource::Worktree(document_path),
        };
        let context = super::production_mutation_context();

        with_mutation_session(&self.exec.storage, &layout, "document add", |session| {
            let initial_paths = BTreeSet::from([VirtualPath::CONFIG, issue_path.clone()]);
            let initial_spec = if skip_scan {
                document_capture_spec(initial_paths)?
            } else {
                document_scan_capture_spec(initial_paths, &source, &BTreeSet::new())?
            };
            let Some(image) = capture_discovering(session, initial_spec, |image, spec| {
                if skip_scan {
                    return Ok(());
                }
                let scan = scan_document_source(&layout, image, path, &source)?;
                discover_scan_assets(spec, &source, &scan.required_paths)
            })?
            else {
                return Ok(SessionStep::Retry);
            };
            let lease_warnings = super::refuse_foreign_leases_held(
                crate::repository_state::assemble_config(&image)?.enforcement_mode(),
                std::slice::from_ref(issue_id),
                claims_guard.as_ref(),
            )?;
            let scan = if skip_scan {
                CapturedDocumentScan::empty()
            } else {
                scan_document_source(&layout, &image, path, &source)?
            };
            let scan = hydrate_document_scan(&layout, &image, &source, scan)?;

            let issue = captured_issue(&image, &issue_path, issue_id)?;
            let derived = derive_document_add(
                issue,
                path,
                commit,
                label,
                doc_type,
                scan.format,
                scan.assets,
            );
            let mut warnings = lease_warnings;
            warnings.extend(
                scan.warning
                    .as_ref()
                    .map(|warning| warning.message().to_string()),
            );
            if derived.intents.is_empty() {
                return Ok(SessionStep::Done((derived.outcome, warnings)));
            }
            let plan = finalize(&layout, &image, &context, &derived.intents, None)?;
            Ok(SessionStep::Apply(plan, (derived.outcome, warnings)))
        })
    }

    /// Remove one path-selected document reference and its event atomically.
    fn publish_document_remove(
        &self,
        issue_id: &IssueId,
        path: &str,
    ) -> Result<(DocumentRemoveResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, VirtualPath};
        use std::collections::BTreeSet;

        let layout = self.exec.require_layout()?;
        let claims_guard = super::claims_mutation_guard(self.exec.require_worktree_paths()?)?;
        let issue_path = VirtualPath::issue_record(issue_id);
        let context = super::production_mutation_context();
        with_mutation_session(&self.exec.storage, &layout, "document removal", |session| {
            let paths = BTreeSet::from([VirtualPath::CONFIG, issue_path.clone()]);
            let Some(image) = capture_or_retry(session.capture(document_capture_spec(paths)?))?
            else {
                return Ok(SessionStep::Retry);
            };
            let warnings = super::refuse_foreign_leases_held(
                crate::repository_state::assemble_config(&image)?.enforcement_mode(),
                std::slice::from_ref(issue_id),
                claims_guard.as_ref(),
            )?;
            let issue = captured_issue(&image, &issue_path, issue_id)?;
            let derived = derive_document_remove(issue, path)?;
            let plan = finalize(&layout, &image, &context, &derived.intents, None)?;
            Ok(SessionStep::Apply(plan, (derived.outcome, warnings)))
        })
    }

    /// `issue_id`'s document at `path`, explicit or derived.
    ///
    /// # Errors
    ///
    /// [`PathReadError::NotFound`](crate::storage::PathReadError::NotFound)
    /// when the issue has no such document.
    fn require_issue_document(
        &self,
        issue_id: &IssueId,
        path: &str,
    ) -> Result<crate::domain::DocumentReference>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let issue = self.exec.storage.load_issue(issue_id)?;
        Ok(self.find_issue_document(issue, path)?)
    }

    /// The code checkout diff reads through.
    fn require_code_checkout(&self) -> Result<crate::storage::git::GitCheckout> {
        use crate::errors::{GitRequirementGap, RequiresGitError};

        Ok(self
            .exec
            .require_worktree_paths()?
            .code_checkout()
            .ok_or_else(|| RequiresGitError::new(GitRequirementGap::NoRepository))?)
    }

    /// Read a document's bytes at `at_commit`, else its pinned commit, else
    /// the working tree.
    pub fn document_content(
        &self,
        issue_id: &IssueId,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<DocumentReading>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let document = self.require_issue_document(issue_id, path)?;
        let reference = at_commit.or(document.commit.as_deref());
        let (bytes, commit) = self
            .exec
            .storage
            .read_path_bytes(&document.path, reference)?;
        Ok(DocumentReading {
            document,
            bytes,
            commit,
        })
    }

    /// The commits that changed a document, newest first; empty without a
    /// Git checkout or before its first commit.
    pub fn document_history(&self, issue_id: &IssueId, path: &str) -> Result<DocumentHistory>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.require_issue_document(issue_id, path)?;
        let commits = match self.exec.require_worktree_paths()?.code_checkout() {
            None => Vec::new(),
            Some(checkout) => match checkout.file_history(path) {
                Ok(commits) => commits,
                // `git log` fails on an unborn branch; only then is a failure empty history.
                Err(error) => match checkout.head() {
                    Ok(None) => Vec::new(),
                    _ => return Err(error.into()),
                },
            },
        }
        .into_iter()
        .map(commit_info_from_git)
        .collect();
        Ok(DocumentHistory {
            path: path.to_string(),
            commits,
        })
    }

    /// A line diff of a document between `from` and `to` (default `HEAD`).
    pub fn document_diff(
        &self,
        issue_id: &IssueId,
        path: &str,
        from: &str,
        to: Option<&str>,
    ) -> Result<DocumentDiffResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.require_issue_document(issue_id, path)?;
        let checkout = self.require_code_checkout()?;
        let layout = self.exec.require_layout()?;
        let paths = self.exec.require_worktree_paths()?;
        let to_ref = to.unwrap_or("HEAD");
        let checkout_top_level = paths
            .code_checkout_top_level()
            .unwrap_or(layout.worktree_root());
        let diff = render_document_diff(
            &checkout,
            checkout_top_level,
            layout.worktree_root(),
            path,
            from,
            to_ref,
        )?;
        Ok(DocumentDiffResult {
            path: path.to_string(),
            from_commit: from.to_string(),
            to_commit: to_ref.to_string(),
            diff,
        })
    }

    /// Every issue's document paths, explicit or derived, sorted and
    /// deduplicated.
    pub fn get_issue_document_paths(&self) -> Result<Vec<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        Ok(self
            .with_issue_documents(self.exec.storage.list_issues()?)?
            .into_iter()
            .flat_map(|(_, documents)| documents)
            .map(|document| document.path)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    /// Read a file as raw bytes from the repository, optionally at a git commit.
    ///
    /// Delegates to `IssueStore::read_path_bytes` so that all filesystem/git I/O
    /// stays in the storage layer.  Returns the raw byte content and a
    /// commit-hash string (`"working-tree"` for filesystem reads).
    ///
    /// Used by the server's raw document endpoints to serve binary-faithful
    /// content without UTF-8 conversion.
    pub fn read_path_bytes(
        &self,
        path: &str,
        at_commit: Option<&str>,
    ) -> Result<(Vec<u8>, String), crate::storage::PathReadError> {
        self.exec.storage.read_path_bytes(path, at_commit)
    }

    pub fn list_document_assets(
        &self,
        issue_id: &IssueId,
        path: &str,
        rescan: bool,
    ) -> Result<(crate::commands::AssetListResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::document::AssetType;
        use anyhow::anyhow;

        let mut warnings = Vec::new();

        let full_id = issue_id.clone();
        let issue = self.exec.storage.load_issue(&full_id)?;

        // Find the document in the issue
        let document = issue
            .documents
            .iter()
            .find(|document| document.path == path)
            .ok_or_else(|| anyhow!("Document '{}' not linked to issue {}", path, issue_id))?;

        // Rescan if requested
        let assets = if rescan {
            self.rescan_document_assets(&full_id, path, &mut warnings)?
        } else {
            document.assets.clone()
        };

        // Categorize assets
        let total = assets.len();
        let per_doc_count = assets
            .iter()
            .filter(|a| !a.is_shared && a.asset_type == AssetType::Local)
            .count();
        let shared_count = assets
            .iter()
            .filter(|a| a.is_shared && a.asset_type == AssetType::Local)
            .count();
        let external_count = assets
            .iter()
            .filter(|a| a.asset_type == AssetType::External)
            .count();
        let missing_count = assets
            .iter()
            .filter(|a| a.asset_type == AssetType::Missing)
            .count();

        let listed = crate::commands::AssetListResult::with_extras(
            assets,
            crate::commands::AssetListScope {
                worktree_root: self.exec.require_layout()?.worktree_root().to_path_buf(),
                issue_id: full_id,
                document_path: path.to_string(),
                summary: crate::commands::AssetSummary {
                    total,
                    per_doc: per_doc_count,
                    shared: shared_count,
                    external: external_count,
                    missing: missing_count,
                },
            },
        );
        Ok((listed, warnings))
    }

    /// Rescan one unpinned document from a coherent captured image and publish
    /// the changed inventory plus its audit event in the same delta. Identical
    /// inventories are a true no-op: no id, timestamp, event, or issue bytes.
    fn rescan_document_assets(
        &self,
        issue_id: &IssueId,
        path: &str,
        warnings: &mut Vec<String>,
    ) -> Result<Vec<crate::document::Asset>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationIntent, VirtualPath};

        let layout = self.exec.require_layout()?;
        let issue_path = VirtualPath::issue_record(issue_id);
        let document_path = VirtualPath::worktree(path)?;
        let source = DocumentScanSource::Worktree(document_path);

        // Operation-scoped so a fresh-session retry cannot resample the update
        // timestamp or audit-event identity.
        let context = super::production_mutation_context();
        with_mutation_session(&self.exec.storage, &layout, "document rescan", |session| {
            let initial = document_scan_capture_spec(
                [issue_path.clone()].into_iter().collect(),
                &source,
                &Default::default(),
            )?;
            let Some(image) = capture_discovering(session, initial, |image, spec| {
                let scan = scan_document_source(&layout, image, path, &source)?;
                discover_scan_assets(spec, &source, &scan.required_paths)
            })?
            else {
                return Ok(SessionStep::Retry);
            };
            let mut issue = captured_issue(&image, &issue_path, issue_id)?;
            let document = issue
                .documents
                .iter_mut()
                .find(|document| document.path == path)
                .ok_or_else(|| anyhow!("document '{path}' changed during rescan"))?;
            let scan = scan_document_source(&layout, &image, path, &source)?;
            if let Some(warning) = scan.warning {
                warnings.push(warning.message().to_string());
                return Ok(SessionStep::Done(match warning {
                    DocumentScanWarning::Missing(_) => document.assets.clone(),
                    DocumentScanWarning::Failed(_) => Vec::new(),
                }));
            }
            let scanned = hydrate_document_scan(&layout, &image, &source, scan)?.assets;
            if document.assets == scanned {
                return Ok(SessionStep::Done(scanned));
            }
            document.assets = scanned.clone();
            let intents = vec![
                MutationIntent::UpdateIssue {
                    issue: Box::new(issue),
                },
                MutationIntent::RecordEvent {
                    phase: 1,
                    event: Box::new(crate::domain::EventBody::draft_issue_updated(
                        issue_id.clone(),
                        "doc-rescan".to_string(),
                        vec!["documents.assets".to_string()],
                    )),
                },
            ];
            let plan = finalize(&layout, &image, &context, &intents, None)?;
            Ok(SessionStep::Apply(plan, scanned))
        })
    }

    fn validate_external_url(agent: &ureq::Agent, url: &str) -> bool {
        agent.head(url).call().is_ok_and(|response| {
            response.status().is_success() || response.status().is_redirection()
        })
    }

    /// Check document links and assets for validity
    ///
    /// Every reference is checked at the version it names: a commit-pinned
    /// reference at its commit — its file, its assets, and its internal links
    /// alike — and an unpinned reference in the working tree. Reference
    /// resolution shares the rule whole-repository validation applies
    /// ([`resolve_document_reference`](crate::document::resolve_document_reference)),
    /// so the two commands classify a reference the same way.
    ///
    /// A pin is a claim about Git history. Where Git cannot resolve it the
    /// reference is reported unresolved, carrying the capture boundary's reason;
    /// unpinned references still resolve from the working tree, so link checking
    /// stays usable in a repository without Git (`@/charter/D-4`).
    pub fn check_document_links(
        &self,
        scope: &crate::domain::issue_scope::IssueScope<crate::domain::identity::IssueId>,
    ) -> Result<crate::commands::LinkCheckResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::document::{AssetType, LinkValidationResult, LinkValidator};
        use crate::domain::issue_scope::{select_issues, IssueScope};
        use crate::storage::git::GitObjectType;
        use std::path::PathBuf;

        let layout = self.exec.require_layout()?;
        let repo_root = layout.worktree_root();

        // Read only the named issue for a single-issue scope.
        let issues = match scope {
            IssueScope::All | IssueScope::Label { .. } => self.exec.storage.list_issues()?,
            IssueScope::Issue(issue_id) => vec![self.exec.storage.load_issue(issue_id)?],
        };

        // Every document in scope, explicit or derived, is checked and tracked.
        let documented = self.with_issue_documents(select_issues(&issues, scope))?;
        let all_documents = documented
            .iter()
            .flat_map(|(issue, documents)| {
                documents
                    .iter()
                    .map(move |document| (issue.id.clone(), document))
            })
            .collect::<Vec<_>>();
        let all_document_paths = all_documents
            .iter()
            .map(|(_, document)| PathBuf::from(&document.path))
            .collect::<Vec<_>>();

        if all_documents.is_empty() {
            return Ok(crate::commands::LinkCheckResult {
                valid: true,
                errors: Vec::new(),
                link_warnings: Vec::new(),
                scope: scope.to_string(),
                summary: crate::commands::LinkCheckSummary {
                    total_documents: 0,
                    valid: 0,
                    errors: 0,
                    warnings: 0,
                },
            });
        }

        // Create link validator
        let link_validator = LinkValidator::new(repo_root.to_path_buf(), all_document_paths);

        // Read every reference at the version it names, before any classification.
        let readings =
            self.read_documents_at_named_versions(&layout, &all_documents, &link_validator)?;

        let missing_worktree_assets = all_documents
            .iter()
            .zip(&readings)
            .filter(|(_, reading)| matches!(reading, CheckedDocument::WorkingTree))
            .flat_map(|((_, document), _)| &document.assets)
            .filter(|asset| asset.asset_type == AssetType::Local)
            .filter_map(|asset| asset.resolved_path.as_ref())
            .filter(|path| !repo_root.join(path).exists())
            .collect::<std::collections::BTreeSet<_>>();
        let committed_assets = match self.exec.require_worktree_paths()?.code_checkout() {
            Some(checkout) if !missing_worktree_assets.is_empty() => {
                let names = missing_worktree_assets
                    .iter()
                    .map(|path| format!("HEAD:./{}", path.to_string_lossy()))
                    .collect::<Vec<_>>();
                missing_worktree_assets
                    .into_iter()
                    .zip(checkout.object_types(names)?)
                    .filter_map(|(path, object_type)| {
                        matches!(object_type, Some(GitObjectType::Blob | GitObjectType::Tree))
                            .then_some(path)
                    })
                    .collect::<std::collections::BTreeSet<_>>()
            }
            _ => std::collections::BTreeSet::new(),
        };

        let mut urls = all_documents
            .iter()
            .zip(&readings)
            .filter(|(_, reading)| !matches!(reading, CheckedDocument::Unresolved(_)))
            .flat_map(|((_, document), _)| &document.assets)
            .filter(|asset| asset.asset_type == AssetType::External)
            .map(|asset| asset.original_path.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|url| (url, false))
            .collect::<Vec<_>>();
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .timeout_global(Some(EXTERNAL_URL_TIMEOUT))
                .build(),
        );
        let chunk_size = urls.len().div_ceil(EXTERNAL_URL_WORKERS).max(1);
        std::thread::scope(|scope| {
            urls.chunks_mut(chunk_size).for_each(|urls| {
                let agent = &agent;
                scope.spawn(move || {
                    urls.iter_mut().for_each(|(url, reachable)| {
                        *reachable = Self::validate_external_url(agent, url);
                    });
                });
            });
        });
        let reachable = urls
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();

        // Validate each document
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        for ((issue_id, doc), reading) in all_documents.iter().zip(&readings) {
            // A pinned reference answers every question about itself from its
            // commit; an unpinned one answers from the working tree.
            let pinned = match reading {
                CheckedDocument::Unresolved(reason) => {
                    errors.push(serde_json::json!({
                        "issue_id": issue_id,
                        "document": doc.path,
                        "type": "missing_document",
                        "message": reason.to_string(),
                    }));
                    continue;
                }
                CheckedDocument::Pinned(pinned) => Some(pinned),
                CheckedDocument::WorkingTree => None,
            };

            // Check assets
            for asset in &doc.assets {
                match asset.asset_type {
                    AssetType::Local => {
                        if let Some(ref resolved) = asset.resolved_path {
                            let present = match pinned {
                                Some(pinned) => pinned.target_kind(resolved).is_file(),
                                None => {
                                    repo_root.join(resolved).exists()
                                        || committed_assets.contains(resolved)
                                }
                            };

                            if !present {
                                errors.push(serde_json::json!({
                                    "issue_id": issue_id,
                                    "document": doc.path,
                                    "type": "missing_asset",
                                    "asset": asset.original_path,
                                    "resolved": resolved.display().to_string(),
                                    "message": format!("Asset not found: {}", asset.original_path),
                                }));
                            } else {
                                // Check if path is risky (deep relative traversal)
                                let parent_count = asset.original_path.matches("../").count();
                                if parent_count >= 2 {
                                    warnings.push(serde_json::json!({
                                        "issue_id": issue_id,
                                        "document": doc.path,
                                        "type": "risky_asset_path",
                                        "asset": asset.original_path,
                                        "message": format!(
                                            "Deep relative path '{}' may break if document is moved",
                                            asset.original_path
                                        ),
                                    }));
                                }
                            }
                        }
                    }
                    AssetType::Missing => {
                        errors.push(serde_json::json!({
                            "issue_id": issue_id,
                            "document": doc.path,
                            "type": "missing_asset",
                            "asset": asset.original_path,
                            "message": format!("Asset classified as missing: {}", asset.original_path),
                        }));
                    }
                    AssetType::Directory => {
                        // Directories are Markdown navigation targets, not assets.
                    }
                    AssetType::Unsupported => {
                        errors.push(serde_json::json!({
                            "issue_id": issue_id,
                            "document": doc.path,
                            "type": "unsupported_asset",
                            "asset": asset.original_path,
                            "message": format!(
                                "Asset resolves to an unsupported artifact type: {}",
                                asset.original_path
                            ),
                        }));
                    }
                    AssetType::External => {
                        if reachable.get(asset.original_path.as_str()) != Some(&true) {
                            errors.push(serde_json::json!({
                                "issue_id": issue_id,
                                "document": doc.path,
                                "type": "unreachable_url",
                                "asset": asset.original_path,
                                "message": format!("External URL is unreachable: {}", asset.original_path),
                            }));
                        }
                    }
                }
            }

            // Check internal document links
            let doc_path_rel = PathBuf::from(&doc.path);
            let scanned = match pinned {
                Some(pinned) => pinned.links(),
                None => link_validator
                    .scan_document_links(&doc_path_rel)
                    .map_err(|error| format!("Failed to scan document for links: {error}")),
            };
            match scanned {
                Ok(links) => {
                    for link in links {
                        let result = match pinned {
                            Some(pinned) => {
                                link_validator.validate_link_at(&doc_path_rel, &link, |target| {
                                    pinned.target_kind(target)
                                })
                            }
                            None => link_validator.validate_link(&doc_path_rel, &link),
                        };
                        match result {
                            LinkValidationResult::Broken { reason } => {
                                errors.push(serde_json::json!({
                                    "issue_id": issue_id,
                                    "document": doc.path,
                                    "type": "broken_link",
                                    "link": link.target,
                                    "line": link.line_number,
                                    "message": reason,
                                }));
                            }
                            LinkValidationResult::Risky { warning } => {
                                warnings.push(serde_json::json!({
                                    "issue_id": issue_id,
                                    "document": doc.path,
                                    "type": "risky_link",
                                    "link": link.target,
                                    "line": link.line_number,
                                    "message": warning,
                                }));
                            }
                            LinkValidationResult::Valid => {}
                        }
                    }
                }
                Err(message) => {
                    warnings.push(serde_json::json!({
                        "issue_id": issue_id,
                        "document": doc.path,
                        "type": "scan_error",
                        "message": message,
                    }));
                }
            }
        }

        // Count unique documents with errors (a document might have multiple errors)
        let mut error_docs = std::collections::HashSet::new();
        for error in &errors {
            if let Some(doc) = error["document"].as_str() {
                error_docs.insert(doc);
            }
        }

        let error_count = error_docs.len();
        let warning_count = warnings.len();

        Ok(crate::commands::LinkCheckResult {
            valid: errors.is_empty(),
            errors,
            link_warnings: warnings,
            scope: scope.to_string(),
            summary: crate::commands::LinkCheckSummary {
                total_documents: all_documents.len(),
                valid: all_documents.len() - error_count,
                errors: error_count,
                warnings: warning_count,
            },
        })
    }

    /// Read every document reference at the version it names, from one closed
    /// image.
    ///
    /// A pinned reference's link targets are only known once its pinned content
    /// has been scanned, so the capture expands once — the same discover-then-
    /// re-capture shape document scanning uses — and a re-scan that reveals a
    /// target the expanded capture does not hold retries the read.
    fn read_documents_at_named_versions(
        &self,
        layout: &crate::repository_state::RepositoryLayout,
        documents: &[(IssueId, &crate::domain::DocumentReference)],
        link_validator: &crate::document::LinkValidator,
    ) -> Result<Vec<CheckedDocument>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        with_read_session(
            &self.exec.storage,
            layout,
            "document link check",
            |session| capture_link_check_readings(session, documents, link_validator),
        )
    }
}

/// The repository declarations a document location is resolved against: the
/// issue-scoped area registry and the type taxonomy an artifact directory is
/// composed from.
pub(super) struct DocumentDeclarations {
    pub(super) documentation: crate::config::DocumentationConfig,
    pub(super) hierarchy: crate::domain::type_taxonomy::HierarchyConfig,
}

impl DocumentDeclarations {
    /// The declarations `config` carries: its documentation section (defaulted
    /// when absent) and the taxonomy its namespace registry defines.
    ///
    /// # Errors
    ///
    /// Returns the hierarchy constructor's error for malformed declarations.
    pub(super) fn from_config(config: &crate::config::JitConfig) -> Result<Self> {
        let namespaces = crate::config_manager::namespaces_from_config(config);
        Ok(Self {
            documentation: config.documentation.clone().unwrap_or_default(),
            hierarchy: crate::domain::type_taxonomy::HierarchyConfig::from_declarations(
                &namespaces,
            )?,
        })
    }

    /// `issue`'s documents: its explicit links, then the files of `files` it
    /// owns by location.
    fn issue_documents(
        &self,
        issue: &crate::domain::Issue,
        files: &[String],
    ) -> Vec<crate::domain::DocumentReference> {
        issue
            .documents
            .iter()
            .cloned()
            .chain(crate::domain::artifact_directory::derive_issue_documents(
                issue,
                files,
                &self.documentation,
                &self.hierarchy,
            ))
            .collect()
    }
}

/// What a captured image holds of the directory trees an issue owns: the files
/// they list, and the kinds and listings a capture must add to read the rest.
#[derive(Default)]
struct OwnedTreeWalk {
    files: Vec<String>,
    kinds: Vec<crate::repository_state::VirtualPath>,
    listings: Vec<crate::repository_state::VirtualPath>,
}

/// Walk the trees `issue` owns under `declarations` as far as `image` captured
/// them, without following a symbolic link: a root that is absent or not a
/// directory lists nothing, and every listed occupant that is not a directory
/// is a file.
fn walk_owned_trees(
    image: &crate::repository_state::RepositoryImage,
    issue: &crate::domain::Issue,
    declarations: &DocumentDeclarations,
) -> Result<OwnedTreeWalk> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};

    let mut pending = crate::domain::artifact_directory::owned_artifact_directories(
        issue,
        &declarations.documentation,
        &declarations.hierarchy,
    )
    .iter()
    .map(|directory| Ok((VirtualPath::worktree(directory)?, true)))
    .collect::<Result<Vec<_>>>()?;
    let mut walk = OwnedTreeWalk::default();
    while let Some((path, is_root)) = pending.pop() {
        if !image.capture_spec().contains_path(&path) {
            walk.kinds.push(path);
            continue;
        }
        match image.entry(&path)? {
            RepositoryEntry::Directory { .. } => match image.directory_listings().get(&path) {
                Some(listing) => pending.extend(
                    listing
                        .names()
                        .iter()
                        .map(|name| {
                            Ok((
                                VirtualPath::worktree(path.relative().as_path().join(name))?,
                                false,
                            ))
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
                None => walk.listings.push(path),
            },
            RepositoryEntry::Absent => {}
            _ if is_root => {}
            _ => walk.files.push(path.relative().as_str().to_string()),
        }
    }
    Ok(walk)
}

/// Add to `spec` what [`captured_issue_documents`] reads of `issue` from
/// `image`: the trees it owns and the references of every document it has.
pub(super) fn discover_issue_documents(
    image: &crate::repository_state::RepositoryImage,
    spec: &mut crate::repository_state::CaptureSpec,
    issue: &crate::domain::Issue,
    config: &crate::config::JitConfig,
) -> Result<()> {
    let declarations = DocumentDeclarations::from_config(config)?;
    let walk = walk_owned_trees(image, issue, &declarations)?;
    // The walk reads as many listings, as deep, as the owned trees hold; a
    // listing's children are one level below it.
    spec.widen_budget(crate::repository_state::CaptureBudget {
        max_listings: spec.listings().len() + walk.listings.len(),
        max_bytes: 0,
        max_depth: walk
            .kinds
            .iter()
            .map(|path| path.relative().depth())
            .chain(walk.listings.iter().map(|path| path.relative().depth() + 1))
            .max()
            .unwrap_or(0),
    });
    spec.discover_kinds(walk.kinds)?;
    spec.discover_listings(walk.listings)?;
    super::validate::discover_document_references(
        &declarations.issue_documents(issue, &walk.files),
        spec,
    )
}

/// `issue`'s documents as `image` holds them under `config`, the image's own
/// configuration: its explicit links, then the files it owns by location.
///
/// # Errors
///
/// When `image` lacks what [`discover_issue_documents`] requests.
pub(super) fn captured_issue_documents(
    image: &crate::repository_state::RepositoryImage,
    issue: &crate::domain::Issue,
    config: &crate::config::JitConfig,
) -> Result<Vec<crate::domain::DocumentReference>> {
    #[cfg(test)]
    crate::resolution_counts::count_in_image(
        crate::resolution_counts::Resolution::Documents,
        image,
    );
    let declarations = DocumentDeclarations::from_config(config)?;
    let walk = walk_owned_trees(image, issue, &declarations)?;
    if !walk.kinds.is_empty() || !walk.listings.is_empty() {
        anyhow::bail!(
            "the captured image does not hold the artifact directories issue {} owns",
            issue.id
        );
    }
    Ok(declarations.issue_documents(issue, &walk.files))
}

/// Capture and read every document reference at the version it names, in one
/// discovering round of `session`. `None` is a retryable conflict.
fn capture_link_check_readings(
    session: &mut (dyn crate::storage::RepositoryReadSession + '_),
    documents: &[(IssueId, &crate::domain::DocumentReference)],
    link_validator: &crate::document::LinkValidator,
) -> Result<Option<Vec<CheckedDocument>>> {
    let spec = link_check_capture_spec(documents, &std::collections::BTreeSet::new())?;
    let Some(image) = capture_discovering(session, spec, |image, spec| {
        pinned_link_targets(image, documents, link_validator)
            .iter()
            .try_for_each(|(revision, path)| discover_derived_pinned(spec, revision, path))
    })?
    else {
        return Ok(None);
    };
    read_documents(&image, documents, link_validator).map(Some)
}

impl CapturedDocumentScan {
    fn empty() -> Self {
        Self {
            format: None,
            assets: Vec::new(),
            required_paths: std::collections::BTreeSet::new(),
            warning: None,
        }
    }
}

/// Repository-relative paths of the local assets `document` records.
fn local_asset_paths(
    document: &crate::domain::DocumentReference,
) -> impl Iterator<Item = &std::path::Path> {
    document
        .assets
        .iter()
        .filter(|asset| matches!(asset.asset_type, crate::document::AssetType::Local))
        .filter_map(|asset| asset.resolved_path.as_deref())
}

/// Declare the evidence a link check reads: the kind of each reference at the
/// version it names, the content of a pinned reference, whose links are scanned
/// from it, and the kinds of a pinned reference's local assets at its commit and
/// of `link_targets` discovered by scanning pinned content.
///
/// An unpinned reference's links and assets are checked against the working
/// tree itself, so its content is never captured.
fn link_check_capture_spec(
    documents: &[(IssueId, &crate::domain::DocumentReference)],
    link_targets: &std::collections::BTreeSet<(String, String)>,
) -> Result<crate::repository_state::CaptureSpec> {
    use crate::document::DocumentReferenceRequests;
    use crate::repository_state::CaptureSpec;
    use std::collections::BTreeSet;

    let mut spec = CaptureSpec::new(Vec::new(), LINK_CHECK_CAPTURE_BUDGET)?;
    let mut worktree = BTreeSet::new();
    for (_, document) in documents {
        let requests = DocumentReferenceRequests::for_reference(document)?;
        worktree.extend(requests.worktree().cloned());
        let (revision, path) = requests.pinned();
        if document.commit.is_some() {
            spec.discover_pinned(revision, path)?;
            for asset in local_asset_paths(document) {
                discover_derived_pinned(&mut spec, revision, &asset.to_string_lossy())?;
            }
        } else {
            spec.discover_pinned_kind(revision, path)?;
        }
    }
    spec.discover_kinds(worktree)?;
    for (revision, path) in link_targets {
        discover_derived_pinned(&mut spec, revision, path)?;
    }
    Ok(spec)
}

/// Request the pinned kind of a path derived from a document's content.
///
/// A derived path the pinned-request contract rejects — a link target that
/// climbs past the repository root, say — names no repository file, so it is
/// dropped from the request set and read as absent rather than failing the whole
/// check. A budget or layout failure still propagates.
fn discover_derived_pinned(
    spec: &mut crate::repository_state::CaptureSpec,
    revision: &str,
    path: &str,
) -> Result<()> {
    use crate::repository_state::CaptureError;

    match spec.discover_pinned_kind(revision, path) {
        Ok(()) | Err(CaptureError::InvalidPinnedRequest(..)) | Err(CaptureError::Layout(_)) => {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// The pinned bytes captured for `(revision, path)`, as text.
fn pinned_text(
    image: &crate::repository_state::RepositoryImage,
    revision: &str,
    path: &str,
) -> std::result::Result<String, String> {
    let bytes = image
        .pinned_evidence()
        .get(&(revision.to_string(), path.to_string()))
        .and_then(crate::repository_state::PinnedDocumentEvidence::bytes)
        .ok_or_else(|| format!("no captured content for '{path}' at '{revision}'"))?;
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|error| error.to_string())
}

/// Captured kind for `(revision, path)`, or missing when it was not requested.
fn pinned_target_kind(
    image: &crate::repository_state::RepositoryImage,
    revision: &str,
    path: &std::path::Path,
) -> crate::document::DocumentTargetKind {
    image
        .pinned_evidence()
        .get(&(revision.to_string(), path.to_string_lossy().into_owned()))
        .map(crate::repository_state::PinnedDocumentEvidence::target_kind)
        .unwrap_or(crate::document::DocumentTargetKind::Missing)
}

/// The link targets every pinned document's captured content names, as pinned
/// requests at that document's own commit.
fn pinned_link_targets(
    image: &crate::repository_state::RepositoryImage,
    documents: &[(IssueId, &crate::domain::DocumentReference)],
    link_validator: &crate::document::LinkValidator,
) -> std::collections::BTreeSet<(String, String)> {
    documents
        .iter()
        .filter_map(|(_, document)| {
            let commit = document.commit.as_deref()?;
            let content = pinned_text(image, commit, &document.path).ok()?;
            let from = std::path::Path::new(&document.path);
            Some(
                crate::document::LinkValidator::scan_links(&content)
                    .into_iter()
                    .filter_map(|link| link_validator.resolve_target(from, &link))
                    .map(move |target| (commit.to_string(), target.to_string_lossy().into_owned())),
            )
        })
        .flatten()
        .collect()
}

/// Derive each reference's reading from one closed image.
fn read_documents(
    image: &crate::repository_state::RepositoryImage,
    documents: &[(IssueId, &crate::domain::DocumentReference)],
    link_validator: &crate::document::LinkValidator,
) -> Result<Vec<CheckedDocument>> {
    documents
        .iter()
        .map(|(_, document)| read_document(image, document, link_validator))
        .collect()
}

fn read_document(
    image: &crate::repository_state::RepositoryImage,
    document: &crate::domain::DocumentReference,
    link_validator: &crate::document::LinkValidator,
) -> Result<CheckedDocument> {
    use crate::document::LinkValidator;

    if let Some(reason) = crate::document::resolve_document_reference(image, document)?.unresolved()
    {
        return Ok(CheckedDocument::Unresolved(reason.clone()));
    }
    let Some(commit) = document.commit.as_deref() else {
        return Ok(CheckedDocument::WorkingTree);
    };

    let from = std::path::Path::new(&document.path);
    let (links, scan_error) = match pinned_text(image, commit, &document.path) {
        Ok(content) => (LinkValidator::scan_links(&content), None),
        Err(error) => (
            Vec::new(),
            Some(format!("Failed to scan document for links: {error}")),
        ),
    };
    let target_kinds = local_asset_paths(document)
        .map(std::path::Path::to_path_buf)
        .chain(
            links
                .iter()
                .filter_map(|link| link_validator.resolve_target(from, link)),
        )
        .map(|path| {
            let kind = pinned_target_kind(image, commit, &path);
            (path, kind)
        })
        .collect();
    Ok(CheckedDocument::Pinned(PinnedDocumentReading {
        links,
        target_kinds,
        scan_error,
    }))
}

fn document_capture_spec(
    paths: std::collections::BTreeSet<crate::repository_state::VirtualPath>,
) -> Result<crate::repository_state::CaptureSpec> {
    use crate::repository_state::{CaptureSpec, VirtualPath};

    let mut spec = CaptureSpec::new(paths, document_capture_budget())?;
    spec.discover_tail(VirtualPath::EVENTS)?;
    Ok(spec)
}

fn document_scan_capture_spec(
    data_paths: std::collections::BTreeSet<crate::repository_state::VirtualPath>,
    source: &DocumentScanSource,
    asset_paths: &std::collections::BTreeSet<crate::repository_state::VirtualPath>,
) -> Result<crate::repository_state::CaptureSpec> {
    let mut spec = document_capture_spec(data_paths)?;
    discover_scan_assets(&mut spec, source, asset_paths)?;
    Ok(spec)
}

/// Add a scan's source document and the assets it requires to `spec`.
fn discover_scan_assets(
    spec: &mut crate::repository_state::CaptureSpec,
    source: &DocumentScanSource,
    asset_paths: &std::collections::BTreeSet<crate::repository_state::VirtualPath>,
) -> Result<()> {
    match source {
        DocumentScanSource::Worktree(document_path) => {
            spec.discover_paths(asset_paths.iter().cloned().chain([document_path.clone()]))?
        }
        DocumentScanSource::Pinned { revision, path } => {
            spec.discover_pinned(revision, path)?;
            for asset_path in asset_paths {
                spec.discover_pinned(revision, asset_path.relative().as_path().to_string_lossy())?;
            }
        }
    }
    Ok(())
}

fn captured_issue(
    image: &crate::repository_state::RepositoryImage,
    issue_path: &crate::repository_state::VirtualPath,
    expected_id: &IssueId,
) -> Result<Issue> {
    use crate::repository_state::RepositoryEntry;

    let issue = match image.entry(issue_path)? {
        RepositoryEntry::File { content, .. } => {
            crate::repository_state::decode_issue_record(issue_path, content.bytes())?
        }
        RepositoryEntry::Absent => {
            return Err(crate::storage::IssueNotFoundError::new(expected_id.clone()).into())
        }
        _ => return Err(anyhow!("captured issue path is not an ordinary file")),
    };
    if &issue.id != expected_id {
        return Err(anyhow!(
            "captured issue identity mismatch: requested {expected_id}, found {}",
            issue.id
        ));
    }
    Ok(issue)
}

/// Discover one document exclusively from the supplied image.
/// The returned paths are the complete local closure observed in that image.
fn scan_document_from_image(
    layout: &crate::repository_state::RepositoryLayout,
    image: &crate::repository_state::RepositoryImage,
    path: &str,
    document_path: &crate::repository_state::VirtualPath,
) -> Result<CapturedDocumentScan> {
    let Some(bytes) = image.file_bytes(document_path)? else {
        return Ok(CapturedDocumentScan {
            warning: Some(DocumentScanWarning::Missing(format!(
                "Could not read document at {path} - skipping asset scan"
            ))),
            ..CapturedDocumentScan::empty()
        });
    };
    scan_document_bytes(layout, path, bytes)
}

fn scan_document_source(
    layout: &crate::repository_state::RepositoryLayout,
    image: &crate::repository_state::RepositoryImage,
    path: &str,
    source: &DocumentScanSource,
) -> Result<CapturedDocumentScan> {
    match source {
        DocumentScanSource::Worktree(document_path) => {
            scan_document_from_image(layout, image, path, document_path)
        }
        DocumentScanSource::Pinned { revision, path } => {
            let evidence = image
                .pinned_evidence()
                .get(&(revision.clone(), path.clone()))
                .ok_or_else(|| anyhow!("pinned evidence for '{path}' at '{revision}' is absent"))?;
            let Some(bytes) = evidence.bytes() else {
                return Ok(CapturedDocumentScan {
                    warning: Some(DocumentScanWarning::Missing(format!(
                        "Could not read document at {path} from {revision} - skipping asset scan"
                    ))),
                    ..CapturedDocumentScan::empty()
                });
            };
            scan_document_bytes(layout, path, bytes)
        }
    }
}

fn scan_document_bytes(
    layout: &crate::repository_state::RepositoryLayout,
    path: &str,
    bytes: &[u8],
) -> Result<CapturedDocumentScan> {
    use crate::document::{AdapterRegistry, AssetScanner};
    use crate::repository_state::VirtualPath;
    use std::collections::BTreeSet;
    use std::path::Path;

    let content = std::str::from_utf8(bytes)
        .map_err(|error| anyhow!("document '{path}' is not UTF-8: {error}"))?;
    let registry = AdapterRegistry::with_builtins();
    let format = registry
        .resolve(path, content)
        .map(|adapter| adapter.id().to_string());
    let Some(_) = format else {
        return Ok(CapturedDocumentScan::empty());
    };
    let scanner = AssetScanner::new(registry, layout.worktree_root());
    let discovered = match scanner.discover_assets(Path::new(path), content) {
        Ok(discovered) => discovered,
        Err(error) => {
            return Ok(CapturedDocumentScan {
                format,
                warning: Some(DocumentScanWarning::Failed(format!(
                    "Failed to scan assets: {error}"
                ))),
                ..CapturedDocumentScan::empty()
            })
        }
    };
    let required_paths = discovered
        .iter()
        .filter_map(|asset| asset.resolved_path.as_ref())
        .map(VirtualPath::worktree)
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(CapturedDocumentScan {
        format,
        assets: discovered,
        required_paths,
        warning: None,
    })
}

fn hydrate_document_scan(
    layout: &crate::repository_state::RepositoryLayout,
    image: &crate::repository_state::RepositoryImage,
    source: &DocumentScanSource,
    mut scan: CapturedDocumentScan,
) -> Result<CapturedDocumentScan> {
    if scan.warning.is_none() {
        match source {
            DocumentScanSource::Worktree(_) => {
                let scanner = crate::document::AssetScanner::new(
                    crate::document::AdapterRegistry::with_builtins(),
                    layout.worktree_root(),
                );
                scan.assets = scanner
                    .hydrate_assets_from_image(scan.assets, image)
                    .map_err(|error| anyhow!("failed to scan assets: {error}"))?;
            }
            DocumentScanSource::Pinned { revision, .. } => {
                use crate::document::AssetType;
                use sha2::{Digest, Sha256};

                for asset in &mut scan.assets {
                    let Some(path) = asset.resolved_path.as_ref() else {
                        continue;
                    };
                    let key = (revision.clone(), path.to_string_lossy().into_owned());
                    let evidence = image.pinned_evidence().get(&key).ok_or_else(|| {
                        anyhow!("pinned evidence for '{}' at '{revision}' is absent", key.1)
                    })?;
                    match evidence.target_kind() {
                        crate::document::DocumentTargetKind::File => {
                            let bytes = evidence.bytes().ok_or_else(|| {
                                anyhow!("pinned file evidence for '{}' has no bytes", key.1)
                            })?;
                            asset.asset_type = AssetType::Local;
                            asset.mime_type = crate::document::AssetScanner::detect_mime_type(path);
                            asset.content_hash = Some(format!("{:x}", Sha256::digest(bytes)));
                        }
                        crate::document::DocumentTargetKind::Directory => {
                            asset.asset_type = AssetType::Directory;
                        }
                        crate::document::DocumentTargetKind::Unsupported => {
                            asset.asset_type = AssetType::Unsupported;
                        }
                        crate::document::DocumentTargetKind::Missing => {}
                    }
                }
            }
        }
    }
    Ok(scan)
}

#[allow(clippy::too_many_arguments)]
fn derive_document_add(
    mut issue: Issue,
    path: &str,
    commit: Option<&str>,
    label: Option<&str>,
    doc_type: Option<&str>,
    format: Option<String>,
    assets: Vec<crate::document::Asset>,
) -> DerivedDocumentMutation<DocumentAddResult> {
    use crate::domain::DocumentReference;
    use crate::repository_state::MutationIntent;

    let existing_index = issue
        .documents
        .iter()
        .position(|document| document.path == path);
    let updated = existing_index.is_some();
    let document = match existing_index {
        Some(index) => {
            let existing = &issue.documents[index];
            DocumentReference {
                path: path.to_string(),
                commit: commit.map(String::from),
                label: label.map(String::from).or_else(|| existing.label.clone()),
                doc_type: doc_type
                    .map(String::from)
                    .or_else(|| existing.doc_type.clone()),
                format,
                assets,
            }
        }
        None => DocumentReference {
            path: path.to_string(),
            commit: commit.map(String::from),
            label: label.map(String::from),
            doc_type: doc_type.map(String::from),
            format,
            assets,
        },
    };
    let outcome = DocumentAddResult {
        issue_id: issue.id.clone(),
        document: document.clone(),
        updated,
    };
    if existing_index.is_some_and(|index| issue.documents[index] == document) {
        return DerivedDocumentMutation {
            outcome,
            intents: Vec::new(),
        };
    }
    match existing_index {
        Some(index) => issue.documents[index] = document,
        None => issue.documents.push(document),
    }
    let event = crate::domain::EventBody::draft_issue_updated(
        issue.id.clone(),
        if updated { "doc-update" } else { "doc-add" }.to_string(),
        vec!["documents".to_string()],
    );
    DerivedDocumentMutation {
        outcome,
        intents: vec![
            MutationIntent::UpdateIssue {
                issue: Box::new(issue),
            },
            MutationIntent::RecordEvent {
                phase: 1,
                event: Box::new(event),
            },
        ],
    }
}

fn derive_document_remove(
    mut issue: Issue,
    path: &str,
) -> Result<DerivedDocumentMutation<DocumentRemoveResult>> {
    use crate::repository_state::MutationIntent;

    if !issue.documents.iter().any(|document| document.path == path) {
        return Err(crate::errors::NotFoundError::new(format!(
            "Document reference {path} not found in issue {}",
            issue.id
        ))
        .into());
    }
    issue.documents.retain(|document| document.path != path);
    let outcome = DocumentRemoveResult {
        issue_id: issue.id.clone(),
        path: path.to_string(),
    };
    let event = crate::domain::EventBody::draft_issue_updated(
        issue.id.clone(),
        "doc-remove".to_string(),
        vec!["documents".to_string()],
    );
    Ok(DerivedDocumentMutation {
        outcome,
        intents: vec![
            MutationIntent::UpdateIssue {
                issue: Box::new(issue),
            },
            MutationIntent::RecordEvent {
                phase: 1,
                event: Box::new(event),
            },
        ],
    })
}

fn commit_info_from_git(commit: crate::storage::git::FileHistoryCommit) -> CommitInfo {
    let datetime = chrono::DateTime::from_timestamp(commit.commit_time(), 0).unwrap_or_default();
    CommitInfo {
        sha: commit.commit().as_str()[..7].to_string(),
        author: commit.author().to_string(),
        date: datetime.format("%Y-%m-%d %H:%M:%S").to_string(),
        message: commit.message().trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{finalize, CaptureSpec, MutationContext, VirtualPath};
    use crate::storage::{
        InMemoryStorage, IssueStore, RepositoryStateStore, RepositoryStateStoreError,
    };
    use crate::test_utils::document_reference;
    use std::collections::BTreeSet;

    /// Explicit commit, then pinned commit, then working tree select the
    /// bytes read, byte for byte.
    #[test]
    fn test_document_content_reads_the_selected_version_bytes() {
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();
        let git = crate::test_harness::git::GitRepository::init(repo.path());
        let path = "artifact.bin";
        let [pinned, explicit, working]: [&[u8]; 3] = [
            b"pinned\0\xff\r\n",
            b"explicit\0\xfe\r\n",
            b"working\0\xfd\r\n",
        ];
        let pin = git.write(path, pinned).commit("Pinned version");
        let at = git.write(path, explicit).commit("Explicit version");
        git.write(path, working);
        let [pinned_issue, unpinned_issue] = [Some(pin.as_str()), None].map(|commit| {
            let id = executor
                .issues()
                .create_issue(
                    "Bytes".into(),
                    String::new(),
                    crate::domain::Priority::Normal,
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .unwrap()
                .issue
                .id;
            executor
                .documents()
                .add_document_reference(&id, path, commit, None, None, true)
                .unwrap();
            id
        });
        let read = |id: &IssueId, at_commit: Option<&str>| {
            executor
                .documents()
                .document_content(id, path, at_commit)
                .unwrap()
        };

        for (reading, bytes, commit) in [
            (read(&pinned_issue, Some(&at)), explicit, &at[..7]),
            (read(&pinned_issue, None), pinned, &pin[..7]),
            (read(&unpinned_issue, None), working, "working-tree"),
        ] {
            assert_eq!(reading.bytes, bytes);
            assert_eq!(reading.commit, commit);
        }
    }

    /// History is an empty list, not an error, without a Git checkout and
    /// before the checkout's first commit.
    #[test]
    fn test_document_history_is_empty_without_git_or_commits() {
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(
            "[worktree]\nenforce_leases = \"off\"\n",
        )
        .unwrap();
        std::fs::write(repo.path().join("notes.md"), "notes").unwrap();
        let id = executor
            .issues()
            .create_issue(
                "History".into(),
                String::new(),
                crate::domain::Priority::Normal,
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap()
            .issue
            .id;
        executor
            .documents()
            .add_document_reference(&id, "notes.md", None, None, None, true)
            .unwrap();
        let history = |executor: &CommandExecutor<crate::storage::JsonFileStorage>| {
            executor
                .documents()
                .document_history(&id, "notes.md")
                .unwrap()
                .commits
        };

        assert!(history(&executor).is_empty());
        crate::test_harness::git::GitRepository::init(repo.path());
        let paths = crate::storage::WorktreePaths::detect_for_data_root(
            &repo.path().join(".jit"),
            repo.path(),
        )
        .unwrap();
        assert!(history(&executor.with_worktree_paths(paths)).is_empty());
    }

    #[test]
    fn test_document_reading_text_replaces_invalid_utf8() {
        let reading = DocumentReading {
            document: document_reference("latin1.txt"),
            bytes: b"caf\xE9".to_vec(),
            commit: "working-tree".to_string(),
        };

        assert_eq!(reading.text(), "caf\u{FFFD}");
    }

    #[test]
    fn test_validate_external_url_times_out_when_server_never_answers() {
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/stalled", listener.local_addr().unwrap());
        let (release, waiting) = std::sync::mpsc::channel();
        let timeout = Duration::from_millis(50);
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .timeout_global(Some(timeout))
                .build(),
        );
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let _ = waiting.recv_timeout(Duration::from_secs(1));
                drop(stream);
            });
            let started = Instant::now();
            let reachable = DocumentOps::<InMemoryStorage>::validate_external_url(&agent, &url);
            let elapsed = started.elapsed();
            let _ = release.send(());
            assert!(!reachable);
            assert!(
                elapsed < Duration::from_millis(500),
                "probe took {elapsed:?}"
            );
        });
    }

    #[test]
    fn test_report_artifact_conformance_propagates_invalid_hierarchy_error() {
        let (repo, storage, _) = crate::test_utils::setup_test_repo_with_taxonomy().unwrap();
        let config = crate::test_taxonomy::invalid_hierarchy_config_fragment();
        std::fs::write(storage.root().join("config.toml"), config).unwrap();
        let executor = crate::test_utils::file_executor(repo.path()).unwrap();

        let error = executor
            .documents()
            .report_artifact_conformance()
            .expect_err("invalid hierarchy must fail artifact conformance reporting");

        assert_eq!(
            error.downcast_ref::<crate::domain::type_taxonomy::ConfigError>(),
            Some(&crate::domain::type_taxonomy::ConfigError::InvalidLevel(0))
        );
    }

    fn capture_spec(paths: impl IntoIterator<Item = VirtualPath>) -> CaptureSpec {
        document_capture_spec(paths.into_iter().collect::<BTreeSet<_>>()).unwrap()
    }

    fn event_bytes(plan: &crate::repository_state::MaterializationPlan) -> Vec<u8> {
        plan.delta().appended_event_bytes().unwrap().to_vec()
    }

    #[test]
    fn test_document_rescan_uses_image_and_identical_inventory_is_noop() {
        let storage = InMemoryStorage::new();
        let mut issue = crate::test_utils::fixture_issue("Docs".into(), String::new());
        issue.documents.push(document_reference("docs/guide.md"));
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        storage.add_worktree_file("docs/guide.md", "![logo](./logo.png)\n");
        storage.add_worktree_file("docs/logo.png", "png bytes");
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(layout)
            .unwrap();

        let first = executor
            .documents()
            .list_document_assets(&id, "docs/guide.md", true)
            .unwrap();
        let first = first.0;
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.items[0].asset_type, crate::document::AssetType::Local);
        let event_count = storage.read_events().unwrap().len();
        let updated_at = storage.load_issue(&id).unwrap().updated_at;

        let second = executor
            .documents()
            .list_document_assets(&id, "docs/guide.md", true)
            .unwrap();
        assert_eq!(second.0.items, first.items);
        assert_eq!(storage.read_events().unwrap().len(), event_count);
        assert_eq!(storage.load_issue(&id).unwrap().updated_at, updated_at);
    }

    #[test]
    fn test_capture_link_check_readings_resolves_unscanned_document_larger_than_capture_budget() {
        let storage = InMemoryStorage::new();
        let max_bytes = 64 * 1024;
        storage.add_worktree_file("receipts/large.log", &"x".repeat(max_bytes * 2));
        let mut issue = crate::test_utils::fixture_issue("Receipts".into(), String::new());
        issue
            .documents
            .push(document_reference("receipts/large.log"));
        crate::commands::test_helpers::seed_issue(&storage, issue.clone());
        let layout = storage.repository_layout();
        let documents = issue
            .documents
            .iter()
            .map(|document| (issue.id.clone(), document))
            .collect::<Vec<_>>();
        let link_validator = crate::document::LinkValidator::new(
            layout.worktree_root().to_path_buf(),
            vec!["receipts/large.log".into()],
        );
        let mut session = crate::commands::test_helpers::LimitedCaptureSession {
            inner: storage.open_mutation_session(layout).unwrap(),
            max_bytes: max_bytes as u64,
        };

        let readings = capture_link_check_readings(&mut session, &documents, &link_validator)
            .unwrap()
            .expect("a settled capture");

        assert!(matches!(
            readings.as_slice(),
            [CheckedDocument::WorkingTree]
        ));
    }

    #[test]
    fn test_check_document_links_for_one_issue_completes_beside_a_held_mutation_session() {
        let executor = crate::commands::test_helpers::setup();
        executor
            .storage()
            .add_worktree_file("docs/guide.md", "# Guide\n");
        let mut issue = crate::test_utils::fixture_issue("Docs".into(), String::new());
        issue.documents.push(document_reference("docs/guide.md"));
        let scope = crate::domain::issue_scope::IssueScope::Issue(issue.id.clone());
        crate::commands::test_helpers::seed_issue(executor.storage(), issue);

        let checked = crate::commands::test_helpers::read_beside_held_mutation_session(
            executor.storage(),
            &executor.require_layout().unwrap(),
            || executor.documents().check_document_links(&scope).unwrap(),
        );

        assert_eq!(checked.summary.total_documents, 1);
        assert!(checked.valid, "{:?}", checked.errors);
    }

    #[test]
    fn test_list_document_references_serializes_the_issue_beside_its_documents() {
        let storage = InMemoryStorage::new();
        let mut issue = crate::test_utils::fixture_issue("Docs".into(), String::new());
        issue.documents.push(document_reference("docs/guide.md"));
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage).with_layout(layout).unwrap();

        let listed = executor.documents().list_document_references(&id).unwrap();
        let value = serde_json::to_value(&listed).unwrap();

        assert_eq!(value["issue_id"], id.as_str());
        assert_eq!(value["documents"][0]["path"], "docs/guide.md");
    }

    #[test]
    fn test_list_document_assets_serializes_its_scope_and_summary_beside_the_assets() {
        let storage = InMemoryStorage::new();
        let mut issue = crate::test_utils::fixture_issue("Docs".into(), String::new());
        issue.documents.push(document_reference("docs/guide.md"));
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        storage.add_worktree_file("docs/guide.md", "![logo](./logo.png)\n");
        storage.add_worktree_file("docs/logo.png", "png bytes");
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage).with_layout(layout).unwrap();

        let listed = executor
            .documents()
            .list_document_assets(&id, "docs/guide.md", true)
            .unwrap();
        let value = serde_json::to_value(&listed.0).unwrap();

        assert_eq!(value["issue_id"], id.as_str());
        assert_eq!(value["document_path"], "docs/guide.md");
        assert_eq!(
            value["summary"]["total"],
            value["assets"].as_array().unwrap().len()
        );
    }

    #[test]
    fn test_document_add_retry_rederives_assets_and_preserves_unrelated_issue_changes() {
        let storage = InMemoryStorage::new();
        let mut issue = crate::test_utils::fixture_issue("Docs".into(), String::new());
        issue.documents.push(document_reference("docs/other.md"));
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        storage.add_worktree_file("docs/guide.md", "![old](./old.png)\n");
        storage.add_worktree_file("docs/old.png", "old bytes");
        storage.add_worktree_file("docs/new.png", "new bytes");

        let layout = storage.repository_layout();
        let issue_path = VirtualPath::issue_record(&id);
        let document_path = VirtualPath::worktree("docs/guide.md").unwrap();
        let old_asset_path = VirtualPath::worktree("docs/old.png").unwrap();
        let expected_time = chrono::DateTime::parse_from_rfc3339("2026-07-20T13:14:15Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let context = MutationContext::deterministic([91; 32], expected_time);

        let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
        let first_image = first_session
            .capture(capture_spec([
                issue_path.clone(),
                document_path.clone(),
                old_asset_path,
            ]))
            .unwrap();
        let first_scan =
            scan_document_from_image(&layout, &first_image, "docs/guide.md", &document_path)
                .unwrap();
        assert!(first_scan
            .required_paths
            .contains(&VirtualPath::worktree("docs/old.png").unwrap()));
        let source = DocumentScanSource::Worktree(document_path.clone());
        let first_scan = hydrate_document_scan(&layout, &first_image, &source, first_scan).unwrap();
        let first = derive_document_add(
            captured_issue(&first_image, &issue_path, &id).unwrap(),
            "docs/guide.md",
            None,
            None,
            None,
            first_scan.format,
            first_scan.assets,
        );
        let first_plan = finalize(&layout, &first_image, &context, &first.intents, None).unwrap();

        let mut concurrent = storage.load_issue(&id).unwrap();
        concurrent.labels.push("owner:concurrent".into());
        crate::commands::test_helpers::seed_issue(&storage, concurrent);
        storage.add_worktree_file("docs/guide.md", "![new](./new.png)\n");
        assert!(matches!(
            first_session.apply(&first_plan),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
        drop(first_session);

        let new_asset_path = VirtualPath::worktree("docs/new.png").unwrap();
        let mut retry_session = storage.open_mutation_session(layout.clone()).unwrap();
        let retry_image = retry_session
            .capture(capture_spec([
                issue_path.clone(),
                document_path.clone(),
                new_asset_path,
            ]))
            .unwrap();
        let retry_scan =
            scan_document_from_image(&layout, &retry_image, "docs/guide.md", &document_path)
                .unwrap();
        assert!(retry_scan
            .required_paths
            .contains(&VirtualPath::worktree("docs/new.png").unwrap()));
        let retry_scan = hydrate_document_scan(&layout, &retry_image, &source, retry_scan).unwrap();
        let retry = derive_document_add(
            captured_issue(&retry_image, &issue_path, &id).unwrap(),
            "docs/guide.md",
            None,
            None,
            None,
            retry_scan.format,
            retry_scan.assets,
        );
        let retry_plan = finalize(&layout, &retry_image, &context, &retry.intents, None).unwrap();
        assert_eq!(event_bytes(&first_plan), event_bytes(&retry_plan));
        retry_session.apply(&retry_plan).unwrap();

        let updated = storage.load_issue(&id).unwrap();
        assert!(updated
            .labels
            .iter()
            .any(|label| label == "owner:concurrent"));
        assert!(updated
            .documents
            .iter()
            .any(|document| document.path == "docs/other.md"));
        let guide = updated
            .documents
            .iter()
            .find(|document| document.path == "docs/guide.md")
            .unwrap();
        assert_eq!(guide.assets.len(), 1);
        assert_eq!(guide.assets[0].original_path, "./new.png");
        assert_eq!(
            guide.assets[0].asset_type,
            crate::document::AssetType::Local
        );
        assert_eq!(updated.updated_at, expected_time);
    }

    #[test]
    fn test_document_remove_from_concurrent_image_preserves_other_fields() {
        let mut issue = crate::test_utils::fixture_issue("Docs".into(), String::new());
        issue.labels.push("owner:concurrent".into());
        issue
            .documents
            .extend(["docs/remove.md", "docs/keep.md"].map(document_reference));

        let derived = derive_document_remove(issue, "docs/remove.md").unwrap();
        let updated = derived
            .intents
            .iter()
            .find_map(|intent| match intent {
                crate::repository_state::MutationIntent::UpdateIssue { issue } => Some(issue),
                _ => None,
            })
            .unwrap();
        assert!(updated
            .labels
            .iter()
            .any(|label| label == "owner:concurrent"));
        assert_eq!(updated.documents.len(), 1);
        assert_eq!(updated.documents[0].path, "docs/keep.md");
    }

    #[test]
    fn test_document_add_preserves_typed_invalid_path_error() {
        let storage = InMemoryStorage::new();
        let issue = crate::test_utils::fixture_issue("Docs".into(), String::new());
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(storage.repository_layout())
            .unwrap();

        let error = executor
            .documents()
            .add_document_reference(&id, "../outside.md", None, None, None, false)
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<crate::storage::PathReadError>(),
            Some(crate::storage::PathReadError::InvalidPath(_))
        ));
        assert!(storage.load_issue(&id).unwrap().documents.is_empty());
        assert!(storage.read_events().unwrap().is_empty());
    }

    /// A repository's accumulated history does not decide whether a document can
    /// be linked: the document capture charges no ceiling the log can outgrow,
    /// so a log past the 64 MiB one it once held still links (jit:48e4f65c).
    #[test]
    fn test_add_document_reference_succeeds_over_an_event_log_past_the_former_ceiling() -> Result<()>
    {
        let (temp, storage) = crate::test_utils::setup_test_repo()?;
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(storage.configured_layout()?)
            .unwrap();
        let issue_id = executor
            .issues()
            .create_issue(
                "Long History Issue".to_string(),
                String::new(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;
        std::fs::create_dir_all(temp.path().join("docs"))?;
        std::fs::write(temp.path().join("docs/plan.md"), "# Plan\n")?;
        crate::test_utils::grow_event_log_beyond(&storage, 64 * 1024 * 1024)?;

        executor.documents().add_document_reference(
            &issue_id,
            "docs/plan.md",
            None,
            None,
            None,
            false,
        )?;

        assert_eq!(
            storage
                .load_issue(&issue_id)?
                .documents
                .iter()
                .map(|document| document.path.as_str())
                .collect::<Vec<_>>(),
            vec!["docs/plan.md"]
        );
        Ok(())
    }

    fn document_lease_fixture(
        mode: &str,
        leased: bool,
        with_reference: bool,
    ) -> (
        CommandExecutor<InMemoryStorage>,
        crate::domain::identity::IssueId,
        Option<crate::domain::Assignee>,
    ) {
        let executor = crate::commands::test_helpers::setup_coordinated_with_enforcement(mode);
        let mut issue =
            crate::test_utils::fixture_issue("Leased document".to_string(), String::new());
        if with_reference {
            issue.documents.push(document_reference("docs/guide.md"));
        }
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(executor.storage(), issue);
        let holder = leased
            .then(|| crate::commands::test_helpers::plant_foreign_lease(&executor, &issue_id));
        (executor, issue_id, holder)
    }

    #[test]
    fn test_document_add_obeys_lease_mode_for_referencing_issue() {
        for (mode, leased) in [("strict", true), ("warn", true), ("strict", false)] {
            let (executor, issue_id, holder) = document_lease_fixture(mode, leased, false);
            let before = executor.storage().load_issue(&issue_id).unwrap();

            let result = executor.documents().add_document_reference(
                &issue_id,
                "docs/guide.md",
                None,
                None,
                None,
                true,
            );

            if mode == "strict" && leased {
                let error = result.unwrap_err();
                assert!(
                    error
                        .downcast_ref::<crate::errors::ForeignLeaseError>()
                        .is_some(),
                    "{error:#}"
                );
                assert_eq!(executor.storage().load_issue(&issue_id).unwrap(), before);
                assert!(executor.storage().read_events().unwrap().is_empty());
            } else {
                let (_, warnings) = result.unwrap();
                if let Some(holder) = holder {
                    assert!(warnings
                        .iter()
                        .any(|warning| warning.contains(&holder.to_string())));
                } else {
                    assert!(warnings.is_empty());
                }
                assert!(executor
                    .storage()
                    .load_issue(&issue_id)
                    .unwrap()
                    .documents
                    .iter()
                    .any(|document| document.path == "docs/guide.md"));
            }
        }
    }

    #[test]
    fn test_document_remove_strict_refuses_another_agents_lease_without_writing() {
        let (executor, issue_id, _) = document_lease_fixture("strict", true, true);
        let before = executor.storage().load_issue(&issue_id).unwrap();

        let error = executor
            .documents()
            .remove_document_reference(&issue_id, "docs/guide.md")
            .unwrap_err();

        assert!(
            error
                .downcast_ref::<crate::errors::ForeignLeaseError>()
                .is_some(),
            "{error:#}"
        );
        assert_eq!(executor.storage().load_issue(&issue_id).unwrap(), before);
        assert!(executor.storage().read_events().unwrap().is_empty());
    }
}
