//! The artifacts this repository generates into its own checkout, and the entry
//! point that brings each one back into agreement with its source.
//!
//! Each artifact is committed, and each carries a drift assertion that fails
//! when the committed bytes differ from what its source now renders. That
//! assertion makes drift visible; the entry point declared here repairs it. Both
//! sides read the render named in [`ArtifactRender::InCrate`], so the repair and
//! the assertion cannot disagree about what the artifact should hold.
//!
//! Every entry point is a script in `scripts/`, named after the artifact, taking
//! no arguments. It prints one `updated: <path>` line per file it rewrote and
//! then one `OK: …` summary; exit 0 means the artifact holds its rendered
//! values, 1 that the render or the publication failed, and 2 that the run was
//! refused or the environment could not support it.
//!
//! [`GENERATED_ARTIFACTS`] is the declaration of the set. The `regenerate`
//! example is the I/O around the renders it names, and `dev/index.md` states the
//! convention for contributors.
//!
//! This module compiles only for the crate's own tests and for the
//! dev-dependency-active builds the example needs, so an adopter build carries
//! none of it.

use crate::profile::template_region;

/// The package whose template declarations the generated registry region holds.
const TEMPLATE_REGION_PACKAGE_ID: &str = "jit-dogfood";

/// A render of one committed artifact's current bytes, given the bytes the
/// checkout holds. A whole-file render ignores them; a region render splices
/// into them, preserving every authored byte outside its delimiters.
pub type RenderFn = fn(&[u8]) -> Result<Vec<u8>, String>;

/// Where an artifact's render lives.
pub enum ArtifactRender {
    /// This crate renders it, into the one file named here. The entry point runs
    /// the `regenerate` example, which calls [`RenderFn`] and publishes the
    /// result; the artifact's drift assertion calls the same function.
    InCrate {
        /// Repository-relative path of the committed file.
        target: &'static str,
        /// The render both the entry point and the drift assertion read.
        render: RenderFn,
    },
    /// The entry point renders it, from values this crate cannot produce in
    /// process — a classification that can only be read out of an already
    /// installed binary, for instance.
    InEntryPoint {
        /// Repository-relative paths of the committed files.
        targets: &'static [&'static str],
    },
}

/// One generated artifact of this repository, and how it is regenerated.
pub struct GeneratedArtifact {
    /// The name its entry point selects it by.
    pub name: &'static str,
    /// The command that regenerates it, as a contributor types it from the
    /// repository root. Drift assertions name this so a failure carries its own
    /// repair.
    pub generator: &'static str,
    /// What the artifact holds, as a noun phrase: "already carries {holds}" and
    /// "rendered from {holds}" are the two reports an entry point prints.
    pub holds: &'static str,
    /// Where the render lives.
    pub render: ArtifactRender,
}

impl GeneratedArtifact {
    /// The committed files this artifact writes.
    pub fn targets(&self) -> Vec<&'static str> {
        match &self.render {
            ArtifactRender::InCrate { target, .. } => vec![target],
            ArtifactRender::InEntryPoint { targets } => targets.to_vec(),
        }
    }
}

/// Every artifact this repository generates into its checkout and commits.
///
/// Each entry cites the path and generator constants its own module declares, so
/// the artifact's drift assertion and this table name the same command.
pub const GENERATED_ARTIFACTS: &[GeneratedArtifact] = &[
    GeneratedArtifact {
        name: "error-code-reference",
        generator: crate::output::test_support::ERROR_CODE_REFERENCE_GENERATOR,
        holds: "the error-code vocabulary declared in `jit::output::ErrorCode`",
        render: ArtifactRender::InCrate {
            target: crate::output::test_support::ERROR_CODE_REFERENCE_PATH,
            render: render_error_code_reference,
        },
    },
    GeneratedArtifact {
        name: "events-reference",
        generator: crate::domain::event_catalog::test_support::REFERENCE_GENERATOR,
        holds: "the event catalog declared in `jit::domain::event_catalog`",
        render: ArtifactRender::InCrate {
            target: crate::domain::event_catalog::test_support::REFERENCE_PATH,
            render: render_events_reference,
        },
    },
    GeneratedArtifact {
        name: "exit-code-reference",
        generator: crate::schema::test_support::EXIT_CODE_REFERENCE_GENERATOR,
        holds: "the exit-code taxonomy declared in `jit::schema`",
        render: ArtifactRender::InCrate {
            target: crate::schema::test_support::EXIT_CODE_REFERENCE_PATH,
            render: render_exit_code_reference,
        },
    },
    GeneratedArtifact {
        name: "gate-presets-reference",
        generator: crate::gate_presets::reference::REFERENCE_GENERATOR,
        holds: "the preset contract and portable checker syntax in `jit::gate_presets`",
        render: ArtifactRender::InCrate {
            target: crate::gate_presets::reference::REFERENCE_PATH,
            render: render_gate_presets_reference,
        },
    },
    GeneratedArtifact {
        name: "runtime-defaults-reference",
        generator: crate::runtime_defaults::REFERENCE_GENERATOR,
        holds: "the defaults declared in `jit::runtime_defaults`",
        render: ArtifactRender::InCrate {
            target: crate::runtime_defaults::REFERENCE_PATH,
            render: render_runtime_defaults_reference,
        },
    },
    GeneratedArtifact {
        name: "shipped-policy-regions",
        generator: "./scripts/generate-shipped-policy-regions.sh",
        holds: "the shipped classification",
        render: ArtifactRender::InEntryPoint {
            targets: &[
                "docs/reference/configuration.md",
                "docs/reference/example-config.toml",
            ],
        },
    },
    GeneratedArtifact {
        name: "storage-records-reference",
        generator: crate::storage::reference::test_support::REFERENCE_GENERATOR,
        holds: "the record layout declared in `jit::storage::reference`",
        render: ArtifactRender::InCrate {
            target: crate::storage::reference::test_support::REFERENCE_PATH,
            render: render_storage_records_reference,
        },
    },
    GeneratedArtifact {
        name: "template-region",
        generator: template_region::TEMPLATE_REGION_GENERATOR,
        holds: "the packaged template declarations",
        render: ArtifactRender::InCrate {
            target: template_region::TEMPLATE_REGISTRY_PATH,
            render: render_template_region,
        },
    },
];

/// The artifact `name` selects, or `None` when no artifact carries that name.
pub fn generated_artifact(name: &str) -> Option<&'static GeneratedArtifact> {
    GENERATED_ARTIFACTS
        .iter()
        .find(|artifact| artifact.name == name)
}

fn render_error_code_reference(_committed: &[u8]) -> Result<Vec<u8>, String> {
    Ok(crate::output::render_error_code_reference().into_bytes())
}

fn render_events_reference(_committed: &[u8]) -> Result<Vec<u8>, String> {
    Ok(crate::domain::event_catalog::render_event_reference().into_bytes())
}

fn render_exit_code_reference(_committed: &[u8]) -> Result<Vec<u8>, String> {
    Ok(crate::schema::render_exit_code_reference().into_bytes())
}

fn render_gate_presets_reference(_committed: &[u8]) -> Result<Vec<u8>, String> {
    Ok(crate::gate_presets::reference::render_reference_markdown().into_bytes())
}

fn render_runtime_defaults_reference(_committed: &[u8]) -> Result<Vec<u8>, String> {
    Ok(crate::runtime_defaults::render_reference_markdown().into_bytes())
}

fn render_storage_records_reference(_committed: &[u8]) -> Result<Vec<u8>, String> {
    crate::storage::reference::render_reference_markdown()
        .map(String::into_bytes)
        .map_err(|error| error.to_string())
}

/// The registry's bytes with the packaged declarations spliced into its
/// generated region.
///
/// The declarations come from this repository's workflow package assembled from
/// the checkout, so a run renders what the checkout currently declares.
///
/// A render that moved a byte outside the delimiters is a defect in the splice
/// rather than an edit to publish, so it is reported instead of returned.
fn render_template_region(committed: &[u8]) -> Result<Vec<u8>, String> {
    let outside = |registry: &[u8]| {
        template_region::outside_template_region(registry).map_err(|error| error.to_string())
    };
    let destination = tempfile::TempDir::new().map_err(|error| error.to_string())?;
    let package = crate::test_utils::assemble_repository_package(
        TEMPLATE_REGION_PACKAGE_ID,
        &destination.path().join(TEMPLATE_REGION_PACKAGE_ID),
    )
    .map_err(|error| error.to_string())?;
    let rendered = template_region::render_template_registry(committed, &package)
        .map_err(|error| error.to_string())?;
    (outside(&rendered)? == outside(committed)?)
        .then_some(rendered)
        .ok_or_else(|| {
            format!(
                "the render changed {} outside the delimiters",
                template_region::TEMPLATE_REGISTRY_PATH
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// The checkout these sources were compiled from.
    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Every declared entry point is a runnable script in the checkout, so a
    /// drift assertion that names one is naming a command a contributor can run.
    #[test]
    fn test_generated_artifacts_declare_a_runnable_entry_point_each() {
        for artifact in GENERATED_ARTIFACTS {
            let relative = artifact.generator.strip_prefix("./").unwrap_or_else(|| {
                panic!(
                    "{}: {} is not repository-relative",
                    artifact.name, artifact.generator
                )
            });
            let path = repository_root().join(relative);
            let metadata = std::fs::metadata(&path).unwrap_or_else(|error| {
                panic!(
                    "{}: {relative} is not in the checkout: {error}",
                    artifact.name
                )
            });
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert!(
                    metadata.permissions().mode() & 0o111 != 0,
                    "{}: {relative} is not executable",
                    artifact.name
                );
            }
            #[cfg(not(unix))]
            assert!(
                metadata.is_file(),
                "{}: {relative} is not a file",
                artifact.name
            );
        }
    }

    /// Every declared target is a committed file, so the set describes the
    /// checkout it regenerates rather than a stale inventory.
    #[test]
    fn test_generated_artifacts_declare_targets_that_exist() {
        for artifact in GENERATED_ARTIFACTS {
            for target in artifact.targets() {
                assert!(
                    repository_root().join(target).is_file(),
                    "{}: {target} is not in the checkout",
                    artifact.name
                );
            }
        }
    }

    /// A name selects one artifact, and a target belongs to one artifact, so no
    /// entry point can shadow another or write over another's output.
    #[test]
    fn test_generated_artifacts_carry_distinct_names_and_targets() {
        let names: std::collections::BTreeSet<_> =
            GENERATED_ARTIFACTS.iter().map(|a| a.name).collect();
        assert_eq!(names.len(), GENERATED_ARTIFACTS.len(), "duplicate name");

        let targets: Vec<_> = GENERATED_ARTIFACTS
            .iter()
            .flat_map(GeneratedArtifact::targets)
            .collect();
        let distinct: std::collections::BTreeSet<_> = targets.iter().collect();
        assert_eq!(distinct.len(), targets.len(), "duplicate target");
    }

    /// Lookup answers by the name the entry points pass, and refuses a name no
    /// artifact carries.
    #[test]
    fn test_generated_artifact_resolves_a_declared_name_only() {
        for artifact in GENERATED_ARTIFACTS {
            assert_eq!(
                generated_artifact(artifact.name).map(|found| found.name),
                Some(artifact.name)
            );
        }
        assert!(generated_artifact("no-such-artifact").is_none());
    }
}
