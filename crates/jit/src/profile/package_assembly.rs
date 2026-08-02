//! Assembly of this repository's workflow package tree.
//!
//! A package is a directory: a manifest, the assets it declares, and a
//! managed-region source. Most of those assets name a repository file as their
//! target and carry that file's bytes, so the repository file is their
//! authority and the tree is produced from it rather than carrying a second
//! copy of it. What the package authors itself — its manifest, its install-only
//! assets, its region source — is checked in, and the assembly draws each
//! declared source from whichever of the two owns it.
//!
//! The production is a repository entry point over the render here, not a build
//! step. No build consumes the assembled tree, so a build step would make every
//! build do work no build consumes, and the directory watching it would need is
//! what once relinked every test target on an unchanged rebuild. Running as an
//! entry point also leaves the manifest one reader, the crate's own package
//! model, where a build script could only have been a second one: a build
//! script cannot import the crate it builds.
//!
//! The destination belongs to the caller, so it carries no properties of its
//! own; what a run guarantees about it is that it replaces it whole. Each run
//! publishes a freshly staged tree, so the result holds exactly what the
//! manifest declares and a source the manifest stops declaring cannot survive
//! into the next run. No run consults what an earlier one left, so no
//! incremental reconciliation, timestamp discipline or watch set is needed.
//!
//! The entry point is [`PACKAGE_ASSEMBLY_ENTRY_POINT`], a thin script over the
//! `assemble-package` example; this module supplies the render it publishes
//! through. Like the other repository-local generator seams it compiles only
//! for the crate's own tests and for the dev-dependency-active builds the
//! example needs, so an adopter build carries none of it.

use super::{
    ProfileManifest, ProfilePackage, ProfilePackageError, JIT_DOGFOOD_LIVE_SOURCE_PREFIX,
    MANIFEST_FILE_NAME,
};
use crate::errors::AlreadyExistsError;
use crate::storage::external_publish::publish_staged_directory_noreplace;
use std::fs;
use std::path::Path;

/// Repository-relative location of the checked-in package sources this
/// repository assembles.
pub const PACKAGE_SOURCE_PATH: &str = "profiles/jit-dogfood";

/// The command that assembles the tree, as a contributor types it from the
/// repository root.
pub const PACKAGE_ASSEMBLY_ENTRY_POINT: &str = "./scripts/assemble-package.sh";

/// Mode a produced file the manifest declares executable is published with.
#[cfg(unix)]
const EXECUTABLE_MODE: u32 = 0o755;

/// Mode every other produced file is published with.
#[cfg(unix)]
const REGULAR_MODE: u32 = 0o644;

/// Why a package tree could not be assembled or published.
#[derive(Debug, thiserror::Error)]
pub enum PackageAssemblyError {
    /// A source the package authors itself is absent or unreadable.
    #[error("package source '{declared}' cannot be read from '{path}': {source}")]
    UnreadablePackageSource {
        /// Package-relative source the manifest declares.
        declared: String,
        /// Path in the checked-in package sources it was read from.
        path: String,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// A declared live asset's repository file is absent or unreadable.
    #[error(
        "live asset source '{declared}' draws from repository file '{path}', \
         which cannot be read: {source}"
    )]
    UnreadableLiveSource {
        /// Package-relative source the manifest declares.
        declared: String,
        /// Repository path the declaration targets.
        path: String,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// The checked-in manifest does not parse, or declares something invalid.
    #[error("invalid package manifest '{path}': {source}")]
    InvalidManifest {
        /// Path of the manifest that was read.
        path: String,
        /// Package-model failure.
        source: ProfilePackageError,
    },
    /// The destination names no parent directory to stage beside.
    #[error("the destination '{0}' has no parent directory to stage beside")]
    RootDestination(String),
    /// Staging or publication filesystem work failed.
    #[error("cannot {action} '{path}': {source}")]
    Publication {
        /// What was being attempted, as a verb phrase.
        action: &'static str,
        /// Path the attempt named.
        path: String,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// The destination was occupied at the moment of publication, so the staged
    /// tree was refused rather than written over what was there.
    #[error(
        "the destination '{path}' was occupied when the assembled tree was \
         published, so nothing was overwritten"
    )]
    DestinationOccupied {
        /// Path the publication refused.
        path: String,
    },
    /// Publishing the staged tree onto the destination failed.
    #[error("cannot publish the assembled package tree at '{path}': {message}")]
    PublicationRefused {
        /// Path the publication named.
        path: String,
        /// What the publisher reported, rendered with its causes.
        message: String,
    },
    /// The staged tree is not a valid package, so it was not published.
    #[error("the staged package tree is not a valid package: {0}")]
    StagedPackage(#[source] ProfilePackageError),
    /// The published tree does not read back as a valid package.
    #[error("the assembled package tree at '{path}' is not a valid package: {source}")]
    PublishedPackage {
        /// Path the tree was published at.
        path: String,
        /// Package-model failure.
        source: ProfilePackageError,
    },
}

/// One file of an assembled package: where it sits in the package, its bytes,
/// and whether the manifest declares it executable.
struct AssembledFile {
    path: String,
    bytes: Vec<u8>,
    executable: bool,
}

/// Assemble the package declared by the sources at `package_source`, drawing
/// every live asset from the repository file under `repository_root` that its
/// declaration targets, and publish the result at `destination`.
///
/// Returns the published tree read back through the package model, so a caller
/// reports what was produced rather than what was intended.
///
/// A previous run's tree is retired into the staging workspace and removed with
/// it, so the destination is taken over rather than merged into. The
/// publication itself is the storage layer's atomic no-replace rename, so a
/// tree that appears between the retire and the publication is reported instead
/// of overwritten, and a destination that survives a run holds one whole tree
/// (`@/invariant/atomic-writes`).
///
/// # Errors
///
/// [`PackageAssemblyError::UnreadableLiveSource`] when a declared live asset's
/// repository file is absent, naming that path;
/// [`PackageAssemblyError::UnreadablePackageSource`] when a source the package
/// authors itself is absent; [`PackageAssemblyError::InvalidManifest`] when the
/// checked-in manifest does not parse or declares something invalid;
/// [`PackageAssemblyError::StagedPackage`] when the assembled tree does not
/// validate as a package, in which case nothing is published;
/// [`PackageAssemblyError::DestinationOccupied`] when the destination is
/// occupied at the moment of publication, which leaves the occupant as it was.
pub fn assemble_package_tree(
    package_source: &Path,
    repository_root: &Path,
    destination: &Path,
) -> Result<ProfilePackage, PackageAssemblyError> {
    let manifest_path = package_source.join(MANIFEST_FILE_NAME);
    let manifest_bytes = read_package_source(MANIFEST_FILE_NAME, &manifest_path)?;
    let manifest = ProfilePackage::parse_manifest(&manifest_bytes).map_err(|source| {
        PackageAssemblyError::InvalidManifest {
            path: manifest_path.display().to_string(),
            source,
        }
    })?;

    let files = drawn_files(&manifest, manifest_bytes, package_source, repository_root)?;
    publish_tree(&files, destination)
}

/// Whether a declared package source draws its bytes from a repository file.
///
/// The prefix is the manifest's own selection rule, stated where the live
/// assets are declared, so which side owns a source is data rather than a
/// second inventory here.
fn is_live_source(declared: &str) -> bool {
    declared.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX)
}

/// Draw every file the manifest declares from the side that owns it.
///
/// A live asset's bytes are its repository file's; every other declared source,
/// the manifest included, is read from the checked-in package sources. A
/// managed region's source is a fragment spliced into its target rather than a
/// copy of it, so it is package-authored whatever prefix it carries.
fn drawn_files(
    manifest: &ProfileManifest,
    manifest_bytes: Vec<u8>,
    package_source: &Path,
    repository_root: &Path,
) -> Result<Vec<AssembledFile>, PackageAssemblyError> {
    let manifest_file = AssembledFile {
        path: MANIFEST_FILE_NAME.to_string(),
        bytes: manifest_bytes,
        executable: false,
    };
    let assets = manifest.assets.iter().map(|asset| {
        let bytes = if is_live_source(&asset.source) {
            read_live_source(&asset.source, &repository_root.join(&asset.target))
        } else {
            read_package_source(&asset.source, &package_source.join(&asset.source))
        }?;
        Ok(AssembledFile {
            path: asset.source.clone(),
            bytes,
            executable: asset.executable,
        })
    });
    let regions = manifest.regions.iter().map(|region| {
        Ok(AssembledFile {
            path: region.source.clone(),
            bytes: read_package_source(&region.source, &package_source.join(&region.source))?,
            executable: false,
        })
    });

    std::iter::once(Ok(manifest_file))
        .chain(assets)
        .chain(regions)
        .collect()
}

/// Read one source the package authors itself.
fn read_package_source(declared: &str, path: &Path) -> Result<Vec<u8>, PackageAssemblyError> {
    fs::read(path).map_err(|source| PackageAssemblyError::UnreadablePackageSource {
        declared: declared.to_string(),
        path: path.display().to_string(),
        source,
    })
}

/// Read the repository file one live asset draws its bytes from.
fn read_live_source(declared: &str, path: &Path) -> Result<Vec<u8>, PackageAssemblyError> {
    fs::read(path).map_err(|source| PackageAssemblyError::UnreadableLiveSource {
        declared: declared.to_string(),
        path: path.display().to_string(),
        source,
    })
}

/// Stage the drawn files, verify the staged tree, and publish it at
/// `destination`.
///
/// The staging workspace is a temporary directory beside the destination, so
/// publication is a rename within one filesystem, and its removal takes the
/// retired tree with it.
fn publish_tree(
    files: &[AssembledFile],
    destination: &Path,
) -> Result<ProfilePackage, PackageAssemblyError> {
    let parent = match destination.parent() {
        Some(parent) if parent.as_os_str().is_empty() => Path::new("."),
        Some(parent) => parent,
        None => {
            return Err(PackageAssemblyError::RootDestination(
                destination.display().to_string(),
            ))
        }
    };
    create_directory("create the destination's parent directory", parent)?;
    let workspace =
        tempfile::TempDir::new_in(parent).map_err(|source| PackageAssemblyError::Publication {
            action: "stage a package tree beside",
            path: parent.display().to_string(),
            source,
        })?;

    let staged = workspace.path().join("staged");
    stage_tree(&staged, files)?;
    // The staged bytes are the bytes the rename publishes, so validating them
    // here is what makes the publication safe rather than hopeful.
    ProfilePackage::from_directory(&staged).map_err(PackageAssemblyError::StagedPackage)?;

    // A previous run's tree is retired into the staging workspace first, so the
    // publication below sees a free name. It is the publication, not this
    // retire, that decides what happens to an occupant: a tree that appears
    // between the two is reported rather than overwritten.
    if fs::symlink_metadata(destination).is_ok() {
        rename(
            "retire the occupied destination",
            destination,
            &workspace.path().join("retired"),
        )?;
    }
    publish_into_free_name(&staged, destination)?;
    drop(workspace);

    ProfilePackage::from_directory(destination).map_err(|source| {
        PackageAssemblyError::PublishedPackage {
            path: destination.display().to_string(),
            source,
        }
    })
}

/// Publish the staged tree onto a destination the caller expects to be free.
///
/// The rename is the storage layer's atomic no-replace publication
/// ([`publish_staged_directory_noreplace`]), which is where this repository's
/// one `renameat2(RENAME_NOREPLACE)` lives, so an occupied destination is
/// reported rather than overwritten (`@/invariant/atomic-writes`). A tree that
/// appeared after the caller retired an occupant therefore survives, and the
/// staged tree stays where it was staged.
fn publish_into_free_name(staged: &Path, destination: &Path) -> Result<(), PackageAssemblyError> {
    publish_staged_directory_noreplace(staged, destination).map_err(|error| {
        let path = destination.display().to_string();
        if error.downcast_ref::<AlreadyExistsError>().is_some() {
            PackageAssemblyError::DestinationOccupied { path }
        } else {
            PackageAssemblyError::PublicationRefused {
                path,
                message: format!("{error:#}"),
            }
        }
    })
}

/// Write the drawn files into a tree rooted at `root`, each with the mode its
/// declaration calls for.
///
/// The executable bit comes from the manifest rather than from the file the
/// bytes were drawn from: the manifest is what an application publishes the
/// mode from, and a separate contract holds each declaration against its
/// repository file's mode.
fn stage_tree(root: &Path, files: &[AssembledFile]) -> Result<(), PackageAssemblyError> {
    files.iter().try_for_each(|file| {
        // Every declared path was validated as a safe relative path when the
        // manifest was parsed, so the join stays inside the staged tree.
        let path = root.join(&file.path);
        create_directory(
            "create the staged package directory",
            path.parent().unwrap_or(root),
        )?;
        fs::write(&path, &file.bytes).map_err(|source| PackageAssemblyError::Publication {
            action: "write the staged package file",
            path: path.display().to_string(),
            source,
        })?;
        set_mode(&path, file.executable)
    })
}

/// Publish `path`'s executable declaration, on the platforms that carry one.
#[cfg(unix)]
fn set_mode(path: &Path, executable: bool) -> Result<(), PackageAssemblyError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable {
        EXECUTABLE_MODE
    } else {
        REGULAR_MODE
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| {
        PackageAssemblyError::Publication {
            action: "set the mode of the staged package file",
            path: path.display().to_string(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _executable: bool) -> Result<(), PackageAssemblyError> {
    Ok(())
}

fn create_directory(action: &'static str, path: &Path) -> Result<(), PackageAssemblyError> {
    fs::create_dir_all(path).map_err(|source| PackageAssemblyError::Publication {
        action,
        path: path.display().to_string(),
        source,
    })
}

fn rename(action: &'static str, from: &Path, to: &Path) -> Result<(), PackageAssemblyError> {
    fs::rename(from, to).map_err(|source| PackageAssemblyError::Publication {
        action,
        path: to.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    /// The checkout these sources were compiled from, which is the repository
    /// whose package the compiled declarations belong to.
    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Every file beneath `root`, by its `root`-relative path, with its bytes
    /// and whether it is executable.
    fn tree_files(root: &Path) -> BTreeMap<String, (Vec<u8>, bool)> {
        fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<String, (Vec<u8>, bool)>) {
            for entry in fs::read_dir(directory).expect("read a tree directory") {
                let path = entry.expect("read a tree entry").path();
                if path.is_dir() {
                    visit(root, &path, files);
                } else {
                    let relative = path
                        .strip_prefix(root)
                        .expect("a path beneath the tree root")
                        .to_string_lossy()
                        .into_owned();
                    let bytes = fs::read(&path).expect("read a tree file");
                    files.insert(relative, (bytes, is_executable(&path)));
                }
            }
        }

        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    #[cfg(unix)]
    fn is_executable(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .expect("stat a tree file")
            .permissions()
            .mode()
            & 0o111
            != 0
    }

    #[cfg(not(unix))]
    fn is_executable(_path: &Path) -> bool {
        false
    }

    /// Write one file, creating its parents, with the mode its role calls for.
    fn write_file(path: &Path, bytes: &[u8], executable: bool) {
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("create a parent");
        fs::write(path, bytes).expect("write a file");
        set_mode(path, executable).expect("set a file mode");
    }

    /// The synthetic manifest: two live assets drawn from a repository, one
    /// install-only asset and one region source the package authors itself.
    const SYNTHETIC_MANIFEST: &str = r#"
[profile]
manifest-version = 1
id = "synthetic-assembly"
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

    /// The manifest declaration of the live asset a test drops to observe a
    /// republished tree.
    const DROPPED_ASSET: &str = r#"
[[asset]]
source = "assets/live/docs/guide.md"
target = "docs/guide.md"
"#;

    /// A checked-in package source tree and the repository its live assets are
    /// drawn from, both under `root`.
    ///
    /// The source tree carries only what the package authors itself, which is
    /// the shape the checked-in copies retire into: a live asset's bytes can
    /// then only have come from the repository file its declaration names.
    fn synthetic_sources(root: &Path, manifest: &str) -> (PathBuf, PathBuf) {
        let package_source = root.join("profiles/synthetic");
        let repository = root.join("repository");
        write_file(
            &package_source.join(MANIFEST_FILE_NAME),
            manifest.as_bytes(),
            false,
        );
        write_file(
            &package_source.join("assets/install/settings.toml"),
            b"[synthetic]\ninstalled = true\n",
            false,
        );
        write_file(
            &package_source.join("assets/regions/guidance.md"),
            b"Synthetic guidance.\n",
            false,
        );
        write_file(
            &repository.join("bin/check.sh"),
            b"#!/bin/sh\nexit 0\n",
            true,
        );
        write_file(
            &repository.join("docs/guide.md"),
            b"# Synthetic guide\n",
            false,
        );
        (package_source, repository)
    }

    /// The produced tree and the checked-in one it will replace, compared while
    /// both exist: the same package-relative paths, the same bytes, and the
    /// same executable declaration. This is the observation the later
    /// retirement of the checked-in copies rests on.
    #[test]
    fn test_assemble_package_tree_produces_the_checked_in_tree_file_for_file() {
        let root = repository_root();
        let temp = TempDir::new().unwrap();
        let destination = temp.path().join("package/jit-dogfood");

        let assembled = assemble_package_tree(&root.join(PACKAGE_SOURCE_PATH), &root, &destination)
            .expect("this repository's package assembles");

        let produced = tree_files(&destination);
        let checked_in = tree_files(&root.join(PACKAGE_SOURCE_PATH));
        assert!(
            produced.len() > 1,
            "the produced tree carries more than a manifest"
        );
        assert_eq!(
            produced.keys().collect::<Vec<_>>(),
            checked_in.keys().collect::<Vec<_>>(),
            "the produced and checked-in trees carry different package-relative paths"
        );
        let differing: Vec<&String> = produced
            .iter()
            .filter(|(path, content)| checked_in.get(*path) != Some(content))
            .map(|(path, _)| path)
            .collect();
        assert_eq!(
            differing,
            Vec::<&String>::new(),
            "each entry names a produced file whose bytes or executable mode \
             differ from the checked-in copy it replaces"
        );
        assert!(
            produced.values().any(|(_, executable)| *executable),
            "no produced file is executable, so the mode half of the comparison \
             observes nothing"
        );
        assert_eq!(assembled.file_count(), produced.len());

        // The live assets' bytes are the repository files', which is what makes
        // the checked-in copies redundant rather than merely equal by habit.
        let live: Vec<&crate::profile::AssetDeclaration> = assembled
            .manifest()
            .assets
            .iter()
            .filter(|asset| is_live_source(&asset.source))
            .collect();
        assert!(!live.is_empty(), "the package declares live assets");
        let drifted: Vec<&str> = live
            .iter()
            .filter(|asset| {
                assembled.source_bytes(&asset.source)
                    != fs::read(root.join(&asset.target)).ok().as_deref()
            })
            .map(|asset| asset.target.as_str())
            .collect();
        assert_eq!(
            drifted,
            Vec::<&str>::new(),
            "each entry names a live asset the assembly did not draw from its \
             repository file"
        );
    }

    /// Each declared source is drawn from the side that owns it: a live asset
    /// from the repository file its declaration targets, everything else from
    /// the checked-in package sources.
    #[test]
    fn test_assemble_package_tree_draws_each_declared_source_from_the_side_that_owns_it() {
        let temp = TempDir::new().unwrap();
        let (package_source, repository) = synthetic_sources(temp.path(), SYNTHETIC_MANIFEST);
        let destination = temp.path().join("out/synthetic");

        let assembled = assemble_package_tree(&package_source, &repository, &destination)
            .expect("the synthetic package assembles");

        // The live sources are absent from the checked-in tree, so the produced
        // bytes can only have come from the repository.
        assert!(!package_source.join("assets/live").exists());
        let produced = tree_files(&destination);
        assert_eq!(
            produced["assets/live/bin/check.sh"].0,
            fs::read(repository.join("bin/check.sh")).unwrap()
        );
        assert_eq!(
            produced["assets/live/docs/guide.md"].0,
            fs::read(repository.join("docs/guide.md")).unwrap()
        );
        assert_eq!(
            produced["assets/install/settings.toml"].0,
            fs::read(package_source.join("assets/install/settings.toml")).unwrap()
        );
        assert_eq!(
            produced["assets/regions/guidance.md"].0,
            fs::read(package_source.join("assets/regions/guidance.md")).unwrap()
        );
        assert_eq!(
            produced[MANIFEST_FILE_NAME].0,
            fs::read(package_source.join(MANIFEST_FILE_NAME)).unwrap()
        );

        // The executable declaration is published, and only where declared.
        let executable: Vec<&String> = produced
            .iter()
            .filter(|(_, (_, executable))| *executable)
            .map(|(path, _)| path)
            .collect();
        assert_eq!(executable, vec!["assets/live/bin/check.sh"]);
        assert_eq!(
            assembled.manifest().profile.id.as_str(),
            "synthetic-assembly"
        );
    }

    /// A declared live source whose repository file is absent fails the run
    /// with a message naming that path, and publishes nothing.
    #[test]
    fn test_assemble_package_tree_reports_a_live_asset_whose_repository_file_is_absent() {
        let temp = TempDir::new().unwrap();
        let (package_source, repository) = synthetic_sources(temp.path(), SYNTHETIC_MANIFEST);
        let destination = temp.path().join("out/synthetic");
        fs::remove_file(repository.join("docs/guide.md")).unwrap();

        let error = assemble_package_tree(&package_source, &repository, &destination)
            .expect_err("a live asset with no repository file cannot be drawn");

        assert!(
            matches!(&error, PackageAssemblyError::UnreadableLiveSource { path, .. }
                if path.ends_with("docs/guide.md")),
            "{error}"
        );
        let message = error.to_string();
        assert!(message.contains("docs/guide.md"), "{message}");
        assert!(
            message.contains("assets/live/docs/guide.md"),
            "the failure names the declaration as well as the path: {message}"
        );
        assert!(
            !destination.exists(),
            "a run that cannot draw a declared source published a tree anyway"
        );
    }

    /// A source the package authors itself is reported the same way, so neither
    /// side of the draw fails silently.
    #[test]
    fn test_assemble_package_tree_reports_a_package_authored_source_that_is_absent() {
        let temp = TempDir::new().unwrap();
        let (package_source, repository) = synthetic_sources(temp.path(), SYNTHETIC_MANIFEST);
        let destination = temp.path().join("out/synthetic");
        fs::remove_file(package_source.join("assets/regions/guidance.md")).unwrap();

        let error = assemble_package_tree(&package_source, &repository, &destination)
            .expect_err("a region source the package does not carry cannot be drawn");

        assert!(
            matches!(&error, PackageAssemblyError::UnreadablePackageSource { declared, .. }
                if declared == "assets/regions/guidance.md"),
            "{error}"
        );
        assert!(!destination.exists());
    }

    /// Each run publishes a freshly staged tree: a package-relative path the
    /// manifest no longer declares is absent from the result, and the staging
    /// the run went through leaves nothing beside the destination.
    #[test]
    fn test_assemble_package_tree_republishes_without_a_source_the_manifest_stopped_declaring() {
        let temp = TempDir::new().unwrap();
        let (package_source, repository) = synthetic_sources(temp.path(), SYNTHETIC_MANIFEST);
        let destination = temp.path().join("out/synthetic");

        assemble_package_tree(&package_source, &repository, &destination)
            .expect("the synthetic package assembles");
        assert!(destination.join("assets/live/docs/guide.md").is_file());

        // The same package, no longer declaring one of its live assets.
        let reduced = SYNTHETIC_MANIFEST.replace(DROPPED_ASSET, "\n");
        assert_ne!(reduced, SYNTHETIC_MANIFEST);
        fs::write(package_source.join(MANIFEST_FILE_NAME), &reduced).unwrap();

        let assembled = assemble_package_tree(&package_source, &repository, &destination)
            .expect("the reduced package assembles over the previous run");

        assert!(
            !destination.join("assets/live/docs/guide.md").exists(),
            "a source the manifest no longer declares survived into the next run"
        );
        assert!(destination.join("assets/live/bin/check.sh").is_file());
        assert!(assembled
            .source_bytes("assets/live/docs/guide.md")
            .is_none());
        assert_eq!(
            tree_files(&destination).keys().collect::<Vec<_>>(),
            vec![
                "assets/install/settings.toml",
                "assets/live/bin/check.sh",
                "assets/regions/guidance.md",
                MANIFEST_FILE_NAME,
            ]
        );

        // The retired tree and the staging workspace go with the run that made
        // them, so the destination's parent holds the destination alone.
        let beside: Vec<String> = fs::read_dir(destination.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(beside, vec!["synthetic"]);
    }

    /// The produced tree validates as a package: every declared source is
    /// present and no produced file is undeclared, which is what the returned
    /// package having been read back from the destination establishes.
    #[test]
    fn test_assemble_package_tree_validates_the_published_tree_as_a_package() {
        let temp = TempDir::new().unwrap();
        let (package_source, repository) = synthetic_sources(temp.path(), SYNTHETIC_MANIFEST);
        let destination = temp.path().join("out/synthetic");

        let assembled = assemble_package_tree(&package_source, &repository, &destination)
            .expect("the synthetic package assembles");

        assert_eq!(
            assembled.source(),
            &crate::profile::ProfilePackageSource::Directory(
                fs::canonicalize(&destination).unwrap()
            ),
            "the returned package is the published tree, not the staged one"
        );
        // Reading the destination again is the same package, so what was
        // returned describes what a later consumer of the tree will read.
        let reread = ProfilePackage::from_directory(&destination)
            .expect("the published tree reads back as a package");
        assert_eq!(reread.hashes(), assembled.hashes());
        assert_eq!(reread.manifest(), assembled.manifest());

        let declared: Vec<&str> = assembled
            .manifest()
            .assets
            .iter()
            .map(|asset| asset.source.as_str())
            .chain(
                assembled
                    .manifest()
                    .regions
                    .iter()
                    .map(|region| region.source.as_str()),
            )
            .chain([MANIFEST_FILE_NAME])
            .collect();
        let produced = tree_files(&destination);
        assert_eq!(produced.len(), declared.len());
        assert!(
            declared.iter().all(|source| produced.contains_key(*source)),
            "the produced tree is missing a declared source"
        );
    }

    /// The publication step refuses an occupied destination rather than
    /// overwriting it, and leaves the staged tree where it was staged.
    ///
    /// This is the state a run reaches when a tree appears between retiring an
    /// occupant and publishing into the freed name: the publication runs
    /// against a destination that is occupied after all. Driving that step
    /// directly observes the refusal without a race to lose.
    #[cfg(target_os = "linux")]
    #[test]
    fn test_publish_into_free_name_refuses_a_destination_occupied_after_the_retire() {
        let temp = TempDir::new().unwrap();
        let staged = temp.path().join("workspace/staged");
        write_file(&staged.join("manifest.toml"), b"staged tree\n", false);
        let destination = temp.path().join("out/synthetic");
        write_file(&destination.join("manifest.toml"), b"the occupant\n", false);

        let error = publish_into_free_name(&staged, &destination)
            .expect_err("an occupied destination is refused");

        assert!(
            matches!(&error, PackageAssemblyError::DestinationOccupied { path }
                if path.ends_with("out/synthetic")),
            "{error}"
        );
        assert_eq!(
            fs::read(destination.join("manifest.toml")).unwrap(),
            b"the occupant\n",
            "the occupant was overwritten"
        );
        assert_eq!(
            fs::read(staged.join("manifest.toml")).unwrap(),
            b"staged tree\n",
            "the staged tree was lost"
        );
    }

    /// The entry point this module names is a runnable script in the checkout,
    /// so the command its documentation states is one a contributor can run.
    #[test]
    fn test_assemble_package_tree_names_a_runnable_entry_point() {
        let entry_point = PACKAGE_ASSEMBLY_ENTRY_POINT
            .strip_prefix("./")
            .expect("the entry point is repository-relative");
        let metadata = fs::metadata(repository_root().join(entry_point))
            .unwrap_or_else(|error| panic!("{entry_point} is not in the checkout: {error}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(
                metadata.permissions().mode() & 0o111 != 0,
                "{entry_point} is not executable"
            );
        }
        #[cfg(not(unix))]
        assert!(metadata.is_file(), "{entry_point} is not a file");
    }

    /// The entry point's documented destination is a path this repository
    /// ignores, and no build script names the assembly or the package sources
    /// it draws from, so no build watches either directory or consumes what a
    /// run produces.
    ///
    /// Being ignored is a property of that documented destination rather than
    /// of destinations in general: a caller names any path it likes, and what
    /// holds of all of them is that a run replaces the destination whole.
    #[test]
    fn test_package_assembly_documents_an_ignored_destination_and_no_build_script_names_it() {
        const DOCUMENTED_DESTINATION: &str = "target/package/jit-dogfood";
        let root = repository_root();

        let entry_point = PACKAGE_ASSEMBLY_ENTRY_POINT
            .strip_prefix("./")
            .expect("the entry point is repository-relative");
        let script = fs::read_to_string(root.join(entry_point)).expect("the entry point is a file");
        assert!(
            script.contains(DOCUMENTED_DESTINATION),
            "{entry_point} no longer names {DOCUMENTED_DESTINATION} as its destination"
        );
        let ignored = Command::new("git")
            .current_dir(&root)
            .args(["check-ignore", "-q", DOCUMENTED_DESTINATION])
            .status()
            .expect("git reports whether a path is ignored");
        assert!(
            ignored.success(),
            "{DOCUMENTED_DESTINATION} is tracked rather than ignored"
        );

        let build_scripts: Vec<PathBuf> = fs::read_dir(root.join("crates"))
            .expect("the workspace has crates")
            .map(|entry| {
                entry
                    .expect("read a crate directory")
                    .path()
                    .join("build.rs")
            })
            .filter(|path| path.is_file())
            .collect();
        assert!(
            !build_scripts.is_empty(),
            "the workspace has build scripts to hold to this"
        );
        let depending: Vec<&PathBuf> = build_scripts
            .iter()
            .filter(|path| {
                let script = fs::read_to_string(path).expect("read a build script");
                [PACKAGE_SOURCE_PATH, "package_assembly", "assemble-package"]
                    .iter()
                    .any(|named| script.contains(named))
            })
            .collect();
        assert_eq!(
            depending,
            Vec::<&PathBuf>::new(),
            "each entry names a build script that reaches the package assembly \
             or the sources it draws from, so a build would depend on what a \
             run produces"
        );
    }
}
