//! Capture of a package tree from the repository files its manifest declares.
//!
//! A package is a directory: a manifest, the assets it declares, and the region
//! sources it declares. An asset declared under [`LIVE_ASSET_SOURCE_PREFIX`]
//! names a repository file as its target and carries that file's bytes, so the
//! repository file is its authority and a capture draws the asset from there.
//! Everything else — the manifest, the install-only assets, the region sources
//! — is content the package itself carries, drawn from the package's own
//! directory. Which side owns a source is therefore data in the manifest rather
//! than an inventory this module keeps.
//!
//! What a capture produces is exactly what the manifest declares: a source the
//! manifest stopped declaring is absent from the result, and the tree is
//! validated as a package before a caller publishes it. Because a live asset's
//! bytes come from the repository file rather than from a checked-in copy, an
//! in-place edit to profile-owned content reaches the package that owns it, and
//! the package identity a capture reports reflects that edit.
//!
//! Every source is read through one no-follow open beneath a handle on the
//! worktree root ([`super::nofollow`]), so that open is where containment is
//! established rather than checked: a resolution leaving the worktree is
//! refused, a link at the name itself is refused, and the bytes and the mode a
//! source is judged against both come from the handle it returns. Whether a
//! source may be read at all is the layout's answer — it resolves each declared
//! target, and one placed outside the worktree is refused before any open.
//!
//! Publication belongs to the caller — this module reads and validates, and the
//! command layer publishes the result through the shared recoverable
//! transaction.

use super::nofollow::{classify_open_failure, is_executable, open_path_nofollow, OpenRefusal};
use super::{
    ProfilePackage, ProfilePackageError, ProfilePackageHashes, ProfilePackageModel,
    LIVE_ASSET_SOURCE_PREFIX, MANIFEST_FILE_NAME,
};
use crate::repository_state::{FileMode, RepositoryLayout, RepositoryLayoutError, VirtualPath};
use cap_std::ambient_authority;
use cap_std::fs::Dir as CapDir;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Which side owns the bytes of one declared package source.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceAuthority {
    /// The repository file the declaration targets, in the adopter-facing
    /// repository-relative spelling.
    Repository(String),
    /// The package's own directory, at the declared package-relative source.
    Package,
}

/// One source a manifest declares, and the side that owns its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeclaredSource {
    /// Package-relative source the manifest declares.
    source: String,
    /// Where a capture draws its bytes from.
    authority: SourceAuthority,
    /// Mode the manifest declares the captured file is published with.
    mode: FileMode,
}

/// One file of a captured package tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFile {
    /// Exact bytes drawn from the owning side.
    pub bytes: Vec<u8>,
    /// Mode the file is published with, taken from its declaration.
    pub mode: FileMode,
}

/// A validated package tree drawn from the sources its manifest declares.
///
/// Holding one means the composed content already validated as a package, so a
/// caller publishes bytes that decode rather than bytes that might.
#[derive(Debug, Clone)]
pub struct CapturedPackageTree {
    model: ProfilePackageModel,
    hashes: ProfilePackageHashes,
    files: BTreeMap<String, CapturedFile>,
}

impl CapturedPackageTree {
    /// Canonical package model the captured manifest declares.
    pub fn model(&self) -> &ProfilePackageModel {
        &self.model
    }

    /// Canonical package and per-repository-target hashes of the captured tree.
    pub fn hashes(&self) -> &ProfilePackageHashes {
        &self.hashes
    }

    /// Every captured file by its package-relative path, in canonical order.
    pub fn files(&self) -> &BTreeMap<String, CapturedFile> {
        &self.files
    }

    /// Captured file count, including `manifest.toml`.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Total captured byte size, including `manifest.toml`.
    pub fn byte_size(&self) -> usize {
        self.files.values().map(|file| file.bytes.len()).sum()
    }
}

/// Why a package tree could not be captured.
#[derive(Debug, thiserror::Error)]
pub enum PackageCaptureError {
    /// A declared source is absent or unreadable.
    #[error("package source '{declared}' cannot be read from '{path}': {source}")]
    UnreadableSource {
        /// Package-relative source the manifest declares.
        declared: String,
        /// Repository-relative path it was read from.
        path: String,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// A declared source is a symbolic link, whose target a capture would
    /// otherwise follow out of the repository.
    #[error("package source '{declared}' draws from '{path}', which is a symbolic link")]
    SymlinkedSource {
        /// Package-relative source the manifest declares.
        declared: String,
        /// Repository-relative path it names.
        path: String,
    },
    /// A declared source is neither absent nor an ordinary file.
    #[error("package source '{declared}' draws from '{path}', which is not an ordinary file")]
    IrregularSource {
        /// Package-relative source the manifest declares.
        declared: String,
        /// Repository-relative path it names.
        path: String,
    },
    /// A declared source resolves outside the repository worktree.
    #[error(
        "package source '{declared}' draws from '{path}', which resolves outside \
         the repository worktree"
    )]
    SourceOutsideWorktree {
        /// Package-relative source the manifest declares.
        declared: String,
        /// Repository-relative path it names.
        path: String,
    },
    /// A declared target is not a path this repository layout can address.
    #[error("package source '{declared}' declares unaddressable target '{target}': {source}")]
    UnaddressableTarget {
        /// Package-relative source the manifest declares.
        declared: String,
        /// Repository-relative target it names.
        target: String,
        /// Layout classification failure.
        source: RepositoryLayoutError,
    },
    /// The captured manifest does not parse, or declares something invalid.
    #[error("invalid package manifest '{path}': {source}")]
    InvalidManifest {
        /// Repository-relative path the manifest was read from.
        path: String,
        /// Package-model failure.
        source: ProfilePackageError,
    },
    /// The captured content is not a valid package, so nothing may be published
    /// from it.
    #[error("the captured package tree is not a valid package: {0}")]
    InvalidCapturedTree(#[source] ProfilePackageError),
}

/// Every source a manifest declares, in canonical package-relative order,
/// paired with the side that owns its bytes.
///
/// The manifest itself is not among them: it is the input a capture reads
/// before it knows what else to draw, not something the manifest declares.
fn declared_sources(model: &ProfilePackageModel) -> Vec<DeclaredSource> {
    model
        .assets
        .iter()
        .map(|asset| DeclaredSource {
            source: asset.source.clone(),
            authority: if asset.source.starts_with(LIVE_ASSET_SOURCE_PREFIX) {
                SourceAuthority::Repository(asset.target.clone())
            } else {
                SourceAuthority::Package
            },
            mode: if asset.executable {
                FileMode::Executable
            } else {
                FileMode::Regular
            },
        })
        // A managed region's source is a fragment spliced into its target
        // rather than a copy of it, so the package authors it whatever prefix
        // it carries.
        .chain(model.regions.iter().map(|region| DeclaredSource {
            source: region.source.clone(),
            authority: SourceAuthority::Package,
            mode: FileMode::Regular,
        }))
        .collect()
}

/// Capture the package whose directory is `package_source`, drawing every live
/// asset from the repository file its declaration targets and everything else
/// from the package's own directory.
///
/// The result holds exactly the manifest, the declared asset sources, and the
/// declared region sources, validated as a package. Nothing is published: a
/// caller decides where the tree goes.
///
/// # Errors
///
/// [`PackageCaptureError::InvalidManifest`] when the package's manifest is
/// absent or does not parse; [`PackageCaptureError::UnreadableSource`] when a
/// declared source is absent, naming both the declaration and the path;
/// [`PackageCaptureError::SymlinkedSource`],
/// [`PackageCaptureError::IrregularSource`] and
/// [`PackageCaptureError::SourceOutsideWorktree`] when a declared source is a
/// symbolic link, is not an ordinary file, or resolves outside the worktree,
/// each raised before any tree is composed;
/// [`PackageCaptureError::InvalidCapturedTree`] when the composed content does
/// not validate as a package, which includes a source carrying executable
/// permission its declaration did not.
pub fn capture_package_tree(
    package_source: &VirtualPath,
    layout: &RepositoryLayout,
) -> Result<CapturedPackageTree, PackageCaptureError> {
    // Every source below is opened through this one handle, so "inside the
    // worktree" is established once, by the directory a capture may read,
    // rather than re-derived from a pathname per source.
    let worktree = CapDir::open_ambient_dir(layout.worktree_root(), ambient_authority()).map_err(
        |source| PackageCaptureError::UnreadableSource {
            declared: MANIFEST_FILE_NAME.to_string(),
            path: layout.worktree_root().display().to_string(),
            source,
        },
    )?;
    let manifest_path = package_relative(package_source, MANIFEST_FILE_NAME).map_err(|source| {
        PackageCaptureError::UnaddressableTarget {
            declared: MANIFEST_FILE_NAME.to_string(),
            target: MANIFEST_FILE_NAME.to_string(),
            source,
        }
    })?;
    let manifest = read_confined(MANIFEST_FILE_NAME, &manifest_path, layout, &worktree)?;
    let model = ProfilePackage::parse_manifest(&manifest.bytes).map_err(|source| {
        PackageCaptureError::InvalidManifest {
            path: manifest_path.repository_relative(),
            source,
        }
    })?;

    let drawn = declared_sources(&model)
        .into_iter()
        .map(|declared| {
            let path = source_path(package_source, &declared, layout)?;
            let read = read_confined(&declared.source, &path, layout, &worktree)?;
            // The published mode is the manifest's declaration, not the mode of
            // the file the bytes came from: the manifest is what an application
            // publishes a target's mode from. The mode that was read still
            // decides validity below, so a source carrying permission its
            // declaration did not is refused rather than quietly narrowed.
            Ok((
                declared.source,
                (
                    CapturedFile {
                        bytes: read.bytes,
                        mode: declared.mode,
                    },
                    read.mode,
                ),
            ))
        })
        .collect::<Result<BTreeMap<String, (CapturedFile, FileMode)>, PackageCaptureError>>()?;

    let observed_executable = drawn
        .iter()
        .filter(|(_, (_, read_mode))| *read_mode == FileMode::Executable)
        .map(|(source, _)| source.clone())
        .collect::<BTreeSet<_>>();
    let files = std::iter::once((
        MANIFEST_FILE_NAME.to_string(),
        CapturedFile {
            bytes: manifest.bytes,
            mode: FileMode::Regular,
        },
    ))
    .chain(drawn.into_iter().map(|(source, (file, _))| (source, file)))
    .collect::<BTreeMap<_, _>>();

    compose_captured_tree(files, observed_executable)
        .map_err(PackageCaptureError::InvalidCapturedTree)
}

/// Validate composed package content and close it into a captured tree.
///
/// The single constructor of a [`CapturedPackageTree`], shared by both routes
/// that compose one — drawing a tree from the repository files a manifest
/// declares, and extracting one from a portable archive — so a tree that exists
/// has passed the same package validation whichever route produced it
/// (`@/inv/convention-convergence`).
///
/// `observed_executable` names the sources whose bytes came from a file
/// carrying executable permission, which is what lets the shared package
/// validation refuse one whose declaration did not.
pub(super) fn compose_captured_tree(
    files: BTreeMap<String, CapturedFile>,
    observed_executable: BTreeSet<String>,
) -> Result<CapturedPackageTree, ProfilePackageError> {
    let bytes = files
        .iter()
        .map(|(source, file)| (source.clone(), file.bytes.clone()))
        .collect::<BTreeMap<_, _>>();
    let (model, hashes) = ProfilePackage::validate_content(&bytes, &observed_executable)?;
    Ok(CapturedPackageTree {
        model,
        hashes,
        files,
    })
}

/// One file as it was read, with the mode the filesystem reported.
struct ReadSource {
    bytes: Vec<u8>,
    mode: FileMode,
}

/// The repository path one declared source draws its bytes from.
fn source_path(
    package_source: &VirtualPath,
    declared: &DeclaredSource,
    layout: &RepositoryLayout,
) -> Result<VirtualPath, PackageCaptureError> {
    match &declared.authority {
        SourceAuthority::Repository(target) => {
            layout
                .classify_repository_relative(target)
                .map_err(|source| PackageCaptureError::UnaddressableTarget {
                    declared: declared.source.clone(),
                    target: target.clone(),
                    source,
                })
        }
        SourceAuthority::Package => {
            package_relative(package_source, &declared.source).map_err(|source| {
                PackageCaptureError::UnaddressableTarget {
                    declared: declared.source.clone(),
                    target: declared.source.clone(),
                    source,
                }
            })
        }
    }
}

/// The repository identity of one package-relative path under `package_source`.
pub(crate) fn package_relative(
    package_source: &VirtualPath,
    relative: &str,
) -> Result<VirtualPath, RepositoryLayoutError> {
    let joined = package_source.relative().as_path().join(relative);
    match package_source.root_class() {
        crate::repository_state::RepositoryRootClass::Worktree => VirtualPath::worktree(joined),
        crate::repository_state::RepositoryRootClass::Data => VirtualPath::data(joined),
    }
}

/// Read one declared source through a single no-follow open beneath the
/// worktree handle.
///
/// The layout says where the source lives and the worktree root says whether a
/// capture may read it, so a source the layout places outside the worktree —
/// content under a data root that is not itself inside the worktree — is
/// refused without being opened at all. Everything after that is the open:
/// `cap_std` refuses a resolution that leaves the handle, so no ancestor can
/// redirect the read out of the worktree, and it refuses a link at the name
/// itself, so a symbolic link is reported rather than dereferenced.
///
/// The bytes and the mode both come from that one opened handle, so they
/// describe one object. Nothing here looks the name up a second time, which is
/// what stops something replacing the name between two lookups from deciding
/// either what a capture reads or what mode it is judged against.
fn read_confined(
    declared: &str,
    path: &VirtualPath,
    layout: &RepositoryLayout,
    worktree: &CapDir,
) -> Result<ReadSource, PackageCaptureError> {
    let refusal = |kind: fn(String, String) -> PackageCaptureError| {
        kind(declared.to_string(), path.repository_relative())
    };
    let unreadable = |source: std::io::Error| PackageCaptureError::UnreadableSource {
        declared: declared.to_string(),
        path: path.repository_relative(),
        source,
    };
    let relative = worktree_relative(path, layout, declared)?;
    let opened =
        open_path_nofollow(worktree, &relative, false).map_err(
            |error| match classify_open_failure(&error) {
                OpenRefusal::Symlink => refusal(symlinked_source),
                OpenRefusal::Escape => refusal(source_outside_worktree),
                OpenRefusal::Unopenable => refusal(irregular_source),
                OpenRefusal::Io => unreadable(error),
            },
        )?;
    let metadata = opened.metadata().map_err(unreadable)?;
    if !metadata.is_file() {
        return Err(refusal(irregular_source));
    }
    let mut bytes = Vec::new();
    (&opened).read_to_end(&mut bytes).map_err(unreadable)?;
    Ok(ReadSource {
        bytes,
        mode: if is_executable(&metadata) {
            FileMode::Executable
        } else {
            FileMode::Regular
        },
    })
}

fn symlinked_source(declared: String, path: String) -> PackageCaptureError {
    PackageCaptureError::SymlinkedSource { declared, path }
}

fn irregular_source(declared: String, path: String) -> PackageCaptureError {
    PackageCaptureError::IrregularSource { declared, path }
}

fn source_outside_worktree(declared: String, path: String) -> PackageCaptureError {
    PackageCaptureError::SourceOutsideWorktree { declared, path }
}

/// Where one declared source sits relative to the worktree root.
///
/// The layout resolves the identity to its physical location — which is how a
/// target under the data root reaches the directory that root actually names —
/// and the worktree root then decides whether a capture may read it. Both are
/// absolute paths the layout already holds, so this derives the name to open
/// rather than consulting the filesystem; the open itself is what enforces
/// containment.
fn worktree_relative(
    path: &VirtualPath,
    layout: &RepositoryLayout,
    declared: &str,
) -> Result<PathBuf, PackageCaptureError> {
    let physical =
        layout
            .resolve(path)
            .map_err(|source| PackageCaptureError::UnaddressableTarget {
                declared: declared.to_string(),
                target: path.repository_relative(),
                source,
            })?;
    physical
        .strip_prefix(layout.worktree_root())
        .map(Path::to_path_buf)
        .map_err(|_| PackageCaptureError::SourceOutsideWorktree {
            declared: declared.to_string(),
            path: path.repository_relative(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::discover_repository_layout;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// A manifest declaring two live assets drawn from repository files, one
    /// install-only asset and one region source the package itself carries.
    const SYNTHETIC_MANIFEST: &str = r#"
[profile]
manifest-version = 1
id = "synthetic-capture"
version = "1.0.0"
jit = ">=1.0.0"

[[live-source]]
root = "bin"
exclude = []

[[live-source]]
root = "docs"
exclude = []

[[asset]]
source = "assets/live/bin/check.sh"
target = "bin/check.sh"
executable = true

[[asset]]
source = "assets/live/docs/guide.md"
target = "docs/guide.md"

[[asset]]
source = "assets/install/settings.toml"
target = ".jit/settings.toml"

[[region]]
source = "assets/regions/guidance.md"
target = "AGENTS.md"
region-id = "guidance"
placement = "append"
"#;

    /// The declaration of the live asset a test drops to observe a capture that
    /// no longer carries it.
    const DROPPED_ASSET: &str = r#"
[[asset]]
source = "assets/live/docs/guide.md"
target = "docs/guide.md"
"#;

    /// Write one file, creating its parents, with the mode its role calls for.
    fn write_file(path: &Path, bytes: &[u8], executable: bool) {
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("create a parent");
        fs::write(path, bytes).expect("write a file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if executable { 0o755 } else { 0o644 };
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set a mode");
        }
        let _ = executable;
    }

    /// A worktree holding a package's checked-in sources and the repository
    /// files its live assets draw from.
    ///
    /// The live sources are absent from the package directory, which is the
    /// shape that makes the origin of a captured live asset observable: its
    /// bytes can only have come from the repository file its declaration names.
    struct Fixture {
        _root: TempDir,
        layout: RepositoryLayout,
        worktree: PathBuf,
        package: VirtualPath,
    }

    impl Fixture {
        fn new(manifest: &str) -> Self {
            let root = TempDir::new().expect("a temporary worktree");
            let worktree = fs::canonicalize(root.path()).expect("resolve the worktree");
            fs::create_dir_all(worktree.join(".jit")).expect("create the data root");
            write_file(
                &worktree.join("profiles/synthetic/manifest.toml"),
                manifest.as_bytes(),
                false,
            );
            write_file(
                &worktree.join("profiles/synthetic/assets/install/settings.toml"),
                b"[synthetic]\ninstalled = true\n",
                false,
            );
            write_file(
                &worktree.join("profiles/synthetic/assets/regions/guidance.md"),
                b"Synthetic guidance.\n",
                false,
            );
            write_file(&worktree.join("bin/check.sh"), b"#!/bin/sh\nexit 0\n", true);
            write_file(
                &worktree.join("docs/guide.md"),
                b"# Synthetic guide\n",
                false,
            );
            let layout = discover_repository_layout(&worktree, worktree.join(".jit"))
                .expect("a repository layout over the fixture");
            Self {
                _root: root,
                layout,
                package: VirtualPath::worktree("profiles/synthetic").expect("a package directory"),
                worktree,
            }
        }

        fn capture(&self) -> Result<CapturedPackageTree, PackageCaptureError> {
            capture_package_tree(&self.package, &self.layout)
        }
    }

    /// Each declared source is drawn from the side that owns it: a live asset
    /// from the repository file its declaration targets, everything else from
    /// the package's own directory, with the manifest alongside them.
    #[test]
    fn test_capture_package_tree_draws_each_declared_source_from_the_side_that_owns_it() {
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);

        let captured = fixture.capture().expect("the synthetic package captures");

        assert!(!fixture
            .worktree
            .join("profiles/synthetic/assets/live")
            .exists());
        let bytes = |source: &str| captured.files()[source].bytes.clone();
        assert_eq!(
            bytes("assets/live/bin/check.sh"),
            fs::read(fixture.worktree.join("bin/check.sh")).unwrap()
        );
        assert_eq!(
            bytes("assets/live/docs/guide.md"),
            fs::read(fixture.worktree.join("docs/guide.md")).unwrap()
        );
        assert_eq!(
            bytes("assets/install/settings.toml"),
            fs::read(
                fixture
                    .worktree
                    .join("profiles/synthetic/assets/install/settings.toml")
            )
            .unwrap()
        );
        assert_eq!(
            bytes("assets/regions/guidance.md"),
            fs::read(
                fixture
                    .worktree
                    .join("profiles/synthetic/assets/regions/guidance.md")
            )
            .unwrap()
        );
        assert_eq!(
            bytes(MANIFEST_FILE_NAME),
            fs::read(fixture.worktree.join("profiles/synthetic/manifest.toml")).unwrap()
        );
        assert_eq!(captured.model().id.as_str(), "synthetic-capture");
    }

    /// The captured tree carries exactly the manifest, the declared asset
    /// sources and the declared region sources, and nothing else.
    #[test]
    fn test_capture_package_tree_carries_exactly_what_the_manifest_declares() {
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);

        let captured = fixture.capture().expect("the synthetic package captures");

        let declared = std::iter::once(MANIFEST_FILE_NAME.to_string())
            .chain(
                declared_sources(captured.model())
                    .into_iter()
                    .map(|declared| declared.source),
            )
            .collect::<BTreeSet<_>>();
        assert_eq!(
            captured.files().keys().cloned().collect::<BTreeSet<_>>(),
            declared,
            "the captured tree carries an undeclared source or omits a declared one"
        );
        assert_eq!(captured.file_count(), declared.len());
    }

    /// A source the manifest stopped declaring is absent from the next capture,
    /// which is what makes the republication whole rather than additive.
    #[test]
    fn test_capture_package_tree_omits_a_source_the_manifest_stopped_declaring() {
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);
        let before = fixture.capture().expect("the synthetic package captures");
        assert!(before.files().contains_key("assets/live/docs/guide.md"));

        let reduced = SYNTHETIC_MANIFEST.replace(DROPPED_ASSET, "\n");
        assert_ne!(reduced, SYNTHETIC_MANIFEST);
        fs::write(
            fixture.worktree.join("profiles/synthetic/manifest.toml"),
            &reduced,
        )
        .unwrap();

        let after = fixture.capture().expect("the reduced package captures");

        assert!(!after.files().contains_key("assets/live/docs/guide.md"));
        assert!(after.files().contains_key("assets/live/bin/check.sh"));
    }

    /// A file mode is published from the declaration that names it, so a
    /// declared-executable asset is captured executable however its repository
    /// file is stored, and nothing else is.
    #[test]
    fn test_capture_package_tree_publishes_the_mode_each_declaration_names() {
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);
        write_file(
            &fixture.worktree.join("bin/check.sh"),
            b"#!/bin/sh\nexit 0\n",
            false,
        );

        let captured = fixture.capture().expect("the synthetic package captures");

        let executable = captured
            .files()
            .iter()
            .filter(|(_, file)| file.mode == FileMode::Executable)
            .map(|(source, _)| source.as_str())
            .collect::<Vec<_>>();
        assert_eq!(executable, vec!["assets/live/bin/check.sh"]);
    }

    /// The mode a source is judged against is the mode of the very object whose
    /// bytes were captured, taken from one opened handle rather than from a
    /// separate look at the name.
    ///
    /// Observed by moving only the mode: the same declaration over the same
    /// unchanged bytes is admitted, refused, and admitted again as the file's
    /// executable bit is set and cleared, and the bytes captured either side of
    /// the refusal are identical. A mode read from anywhere other than the
    /// source that supplied those bytes could not track that.
    #[cfg(unix)]
    #[test]
    fn test_capture_package_tree_judges_the_mode_of_the_source_whose_bytes_it_captured() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);
        let source = fixture.worktree.join("docs/guide.md");
        let set_mode = |mode: u32| {
            fs::set_permissions(&source, fs::Permissions::from_mode(mode)).unwrap();
        };

        let before = fixture.capture().expect("a regular source is admitted");
        set_mode(0o755);
        let refused = fixture
            .capture()
            .expect_err("the same bytes carrying an undeclared executable bit are refused");
        set_mode(0o644);
        let after = fixture.capture().expect("clearing the bit admits it again");

        assert!(
            matches!(
                &refused,
                PackageCaptureError::InvalidCapturedTree(
                    ProfilePackageError::UndeclaredExecutable { path }
                ) if path == "assets/live/docs/guide.md"
            ),
            "{refused}"
        );
        assert_eq!(
            before.files()["assets/live/docs/guide.md"].bytes,
            after.files()["assets/live/docs/guide.md"].bytes,
            "the mode decision moved without the bytes moving with it"
        );
        assert_eq!(before.hashes().package, after.hashes().package);
    }

    /// A repository file carrying executable permission its declaration did not
    /// is refused, so a capture cannot quietly widen what a package publishes.
    #[test]
    fn test_capture_package_tree_refuses_a_source_carrying_undeclared_executable_permission() {
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);
        write_file(
            &fixture.worktree.join("docs/guide.md"),
            b"# Synthetic guide\n",
            true,
        );

        let error = fixture
            .capture()
            .expect_err("an undeclared executable is refused");

        assert!(
            matches!(
                &error,
                PackageCaptureError::InvalidCapturedTree(ProfilePackageError::UndeclaredExecutable {
                    path
                }) if path == "assets/live/docs/guide.md"
            ),
            "{error}"
        );
    }

    /// A declared target that is a symbolic link is refused rather than
    /// dereferenced, so a link cannot decide what a package carries.
    #[cfg(unix)]
    #[test]
    fn test_capture_package_tree_refuses_a_declared_target_that_is_a_symlink() {
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);
        let target = fixture.worktree.join("docs/guide.md");
        fs::write(fixture.worktree.join("docs/real.md"), b"# Elsewhere\n").unwrap();
        fs::remove_file(&target).unwrap();
        std::os::unix::fs::symlink("real.md", &target).unwrap();

        let error = fixture
            .capture()
            .expect_err("a symlinked target is refused");

        assert!(
            matches!(&error, PackageCaptureError::SymlinkedSource { declared, path }
                if declared == "assets/live/docs/guide.md" && path == "docs/guide.md"),
            "{error}"
        );
    }

    /// A declared target reached through a link out of the worktree is refused,
    /// so nothing outside the repository reaches a package.
    #[cfg(unix)]
    #[test]
    fn test_capture_package_tree_refuses_a_declared_target_resolving_outside_the_worktree() {
        let outside = TempDir::new().expect("a directory outside the worktree");
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);
        fs::write(outside.path().join("guide.md"), b"# Outside\n").unwrap();
        fs::remove_dir_all(fixture.worktree.join("docs")).unwrap();
        std::os::unix::fs::symlink(outside.path(), fixture.worktree.join("docs")).unwrap();

        let error = fixture
            .capture()
            .expect_err("a target outside the worktree is refused");

        assert!(
            matches!(&error, PackageCaptureError::SourceOutsideWorktree { path, .. }
                if path == "docs/guide.md"),
            "{error}"
        );
    }

    /// A declared source that is not an ordinary file is refused, naming it.
    #[test]
    fn test_capture_package_tree_refuses_a_declared_source_that_is_not_an_ordinary_file() {
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);
        fs::remove_file(fixture.worktree.join("docs/guide.md")).unwrap();
        fs::create_dir(fixture.worktree.join("docs/guide.md")).unwrap();

        let error = fixture
            .capture()
            .expect_err("a directory where a file was declared is refused");

        assert!(
            matches!(&error, PackageCaptureError::IrregularSource { declared, .. }
                if declared == "assets/live/docs/guide.md"),
            "{error}"
        );
    }

    /// A declared source with no file behind it names both the declaration and
    /// the path it was drawn from, on either side of the draw.
    #[test]
    fn test_capture_package_tree_reports_an_absent_source_on_either_side_of_the_draw() {
        let live = Fixture::new(SYNTHETIC_MANIFEST);
        fs::remove_file(live.worktree.join("docs/guide.md")).unwrap();

        let error = live
            .capture()
            .expect_err("an absent live source is refused");

        assert!(
            matches!(&error, PackageCaptureError::UnreadableSource { declared, path, .. }
                if declared == "assets/live/docs/guide.md" && path == "docs/guide.md"),
            "{error}"
        );

        let authored = Fixture::new(SYNTHETIC_MANIFEST);
        fs::remove_file(
            authored
                .worktree
                .join("profiles/synthetic/assets/regions/guidance.md"),
        )
        .unwrap();

        let error = authored
            .capture()
            .expect_err("an absent package-authored source is refused");

        assert!(
            matches!(&error, PackageCaptureError::UnreadableSource { declared, .. }
                if declared == "assets/regions/guidance.md"),
            "{error}"
        );
    }

    /// Capturing twice from unchanged sources answers one identity, and an
    /// in-place edit to an owned repository file changes it — which is what
    /// makes re-applying a captured package reconcile that edit.
    #[test]
    fn test_capture_package_tree_identity_follows_an_edit_to_an_owned_repository_file() {
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);

        let first = fixture.capture().expect("the synthetic package captures");
        let repeated = fixture.capture().expect("the same sources capture again");
        assert_eq!(first.hashes().package, repeated.hashes().package);
        assert_eq!(first.hashes().targets, repeated.hashes().targets);

        fs::write(
            fixture.worktree.join("docs/guide.md"),
            b"# Edited in place\n",
        )
        .unwrap();
        let edited = fixture.capture().expect("the edited package captures");

        assert_ne!(
            first.hashes().package,
            edited.hashes().package,
            "an edit to an owned repository file left the package identity unchanged"
        );
        assert_eq!(
            edited.files()["assets/live/docs/guide.md"].bytes,
            b"# Edited in place\n"
        );
    }

    /// A package directory with no manifest is reported as an invalid manifest
    /// naming the path, rather than as an empty package.
    #[test]
    fn test_capture_package_tree_reports_a_package_directory_with_no_manifest() {
        let fixture = Fixture::new(SYNTHETIC_MANIFEST);
        fs::remove_file(fixture.worktree.join("profiles/synthetic/manifest.toml")).unwrap();

        let error = fixture
            .capture()
            .expect_err("a package directory with no manifest is refused");

        assert!(
            matches!(&error, PackageCaptureError::UnreadableSource { path, .. }
                if path == "profiles/synthetic/manifest.toml"),
            "{error}"
        );
    }
}
