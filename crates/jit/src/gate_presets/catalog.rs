use super::{load_presets_from_custom_files, GatePresetDefinition, PresetInfo};
use crate::domain::identity::GateKey;
use crate::repository_state::{CaptureError, RepositoryEntry, RepositoryImage, VirtualPath};
use crate::storage::PresetNotFoundError;
use anyhow::Result;
use std::collections::HashMap;

/// A project's gate presets parsed from one captured repository image.
#[derive(Debug)]
pub struct GatePresetCatalog {
    presets: HashMap<GateKey, GatePresetDefinition>,
}

impl GatePresetCatalog {
    /// JSON paths named by the captured preset-directory listing, for capture discovery.
    pub fn paths(image: &RepositoryImage) -> Result<Vec<VirtualPath>> {
        image
            .listed_json_files(&VirtualPath::GATE_PRESETS)
            .map_err(|error| match error {
                CaptureError::NotADirectory(directory) => {
                    crate::errors::InvalidArgumentError::new(format!(
                        "Custom preset root '{}' must be an ordinary directory",
                        directory.relative().as_str()
                    ))
                    .into()
                }
                error => error.into(),
            })
    }

    /// Parse every listed JSON file; refuse absent files and non-file occupants.
    pub fn from_image(image: &RepositoryImage) -> Result<Self> {
        let files = Self::paths(image)?
            .into_iter()
            .map(|path| match image.entry(&path)? {
                RepositoryEntry::File { content, .. } => Ok((
                    path.relative().as_str().to_string(),
                    content.bytes().to_vec(),
                )),
                _ => Err(crate::errors::InvalidArgumentError::new(format!(
                    "Custom preset file '{}' must be an ordinary file",
                    path.relative().as_str()
                ))
                .into()),
            })
            .collect::<Result<Vec<_>>>()?;
        let presets = load_presets_from_custom_files(files)?
            .into_iter()
            .map(|(name, preset)| Ok((name.parse()?, preset)))
            .collect::<Result<_>>()?;
        Ok(Self { presets })
    }

    /// Preset metadata in name order.
    pub fn list(&self) -> Vec<PresetInfo> {
        let mut listed = self
            .presets
            .values()
            .map(|preset| PresetInfo {
                name: preset.name.clone(),
                description: preset.description.clone(),
                gate_count: preset.gates.len(),
            })
            .collect::<Vec<_>>();
        listed.sort_by(|left, right| left.name.cmp(&right.name));
        listed
    }

    /// Look up a preset, returning [`PresetNotFoundError`] when absent.
    pub fn get(&self, name: &str) -> Result<&GatePresetDefinition, PresetNotFoundError> {
        name.parse::<GateKey>()
            .ok()
            .and_then(|key| self.presets.get(&key))
            .ok_or_else(|| PresetNotFoundError::new(name))
    }

    pub(crate) fn definitions(&self) -> &HashMap<GateKey, GatePresetDefinition> {
        &self.presets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::{GateMode, GateStage};
    use crate::gate_presets::{GatePresetDefinition, GateTemplate};
    use crate::repository_state::{
        CaptureBudget, CaptureSpec, CapturedBytes, DirectoryListing, EntryIdentity, FileMode,
        RepositoryEntry, RepositoryImage, RepositoryLayout, RepositoryRootEvidence, VirtualPath,
    };
    use std::collections::BTreeMap;

    fn directory() -> RepositoryEntry {
        RepositoryEntry::Directory {
            identity: EntryIdentity::for_bytes("directory", b"").unwrap(),
            mode: FileMode::Regular,
        }
    }

    fn file(bytes: Vec<u8>) -> RepositoryEntry {
        RepositoryEntry::File {
            content: CapturedBytes::new("file", bytes).unwrap(),
            mode: FileMode::Regular,
        }
    }

    fn preset(name: &str, count: usize) -> GatePresetDefinition {
        GatePresetDefinition {
            name: name.into(),
            description: format!("{name} checks"),
            gates: (0..count)
                .map(|index| GateTemplate {
                    key: format!("check-{index}").parse().unwrap(),
                    title: format!("Check {index}"),
                    description: String::new(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Manual,
                    checker: None,
                })
                .collect(),
        }
    }

    fn image(root: RepositoryEntry, files: Vec<(&str, RepositoryEntry)>) -> RepositoryImage {
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap();
        let dir = VirtualPath::GATE_PRESETS;
        let listing =
            DirectoryListing::new(files.iter().map(|(name, _)| name.to_string())).unwrap();
        let entries = std::iter::once((dir.clone(), root))
            .chain(files.into_iter().map(|(name, entry)| {
                (
                    VirtualPath::data(format!("{}/{name}", dir.relative().as_str())).unwrap(),
                    entry,
                )
            }))
            .collect::<BTreeMap<_, _>>();
        let mut spec = CaptureSpec::new(
            entries.keys().cloned(),
            CaptureBudget {
                max_listings: 1,
                max_bytes: 65536,
                max_depth: 8,
            },
        )
        .unwrap();
        spec.discover_listings([dir.clone()]).unwrap();
        RepositoryImage::close(
            layout,
            spec,
            entries,
            BTreeMap::from([(dir, listing)]),
            BTreeMap::new(),
            &Default::default(),
        )
        .unwrap()
    }

    #[test]
    fn test_catalog_missing_directory_lists_no_presets() {
        let catalog =
            GatePresetCatalog::from_image(&image(RepositoryEntry::Absent, vec![])).unwrap();
        assert!(catalog.list().is_empty());
    }

    #[test]
    fn test_catalog_lists_presets_sorted_with_their_gate_counts() {
        let presets = [preset("zeta", 2), preset("alpha", 1)];
        let captured = image(
            directory(),
            vec![
                ("zeta.json", file(serde_json::to_vec(&presets[0]).unwrap())),
                ("alpha.json", file(serde_json::to_vec(&presets[1]).unwrap())),
                ("notes.txt", file(b"not a preset".to_vec())),
            ],
        );
        let catalog = GatePresetCatalog::from_image(&captured).unwrap();
        let mut expected = presets.iter().collect::<Vec<_>>();
        expected.sort_by_key(|preset| &preset.name);
        let listed = catalog.list();
        assert_eq!(listed.len(), expected.len());
        for (info, preset) in listed.iter().zip(expected) {
            assert_eq!(info.name, preset.name);
            assert_eq!(info.description, preset.description);
            assert_eq!(info.gate_count, preset.gates.len());
            assert_eq!(catalog.get(&info.name).unwrap(), preset);
        }
    }

    #[test]
    fn test_catalog_lookup_absent_preset_returns_preset_not_found() {
        let catalog = GatePresetCatalog::from_image(&image(directory(), vec![])).unwrap();
        let error: crate::storage::PresetNotFoundError = catalog.get("absent").unwrap_err();
        assert!(error.to_string().contains("absent"));
    }

    #[test]
    fn test_catalog_rejects_non_file_json_occupants() {
        let occupants = [
            directory(),
            RepositoryEntry::Symlink {
                target: CapturedBytes::new("link", b"elsewhere.json".to_vec()).unwrap(),
                mode: FileMode::Regular,
            },
            RepositoryEntry::Unsupported {
                identity: EntryIdentity::for_bytes("special", b"").unwrap(),
                reason: "special file".into(),
                mode: FileMode::Regular,
            },
        ];
        for occupant in occupants {
            let captured = image(directory(), vec![("unsafe.json", occupant)]);
            let error = GatePresetCatalog::from_image(&captured).unwrap_err();
            assert!(error.to_string().contains("must be an ordinary file"));
        }
    }

    #[test]
    fn test_catalog_rejects_non_directory_preset_root() {
        let captured = image(file(vec![]), vec![]);
        let error = GatePresetCatalog::from_image(&captured).unwrap_err();
        assert!(error.to_string().contains("must be an ordinary directory"));
    }

    #[test]
    fn test_catalog_reports_invalid_json_in_filename_order() {
        let captured = image(
            directory(),
            vec![
                ("zeta.json", file(b"invalid".to_vec())),
                ("alpha.json", file(b"invalid".to_vec())),
            ],
        );
        let error = GatePresetCatalog::from_image(&captured).unwrap_err();
        assert!(error
            .to_string()
            .contains("Failed to parse custom gate preset file"));
        assert!(error.to_string().contains("alpha.json"));
    }

    #[test]
    fn test_catalog_rejects_filename_stem_different_from_embedded_name() {
        let captured = image(
            directory(),
            vec![(
                "different.json",
                file(serde_json::to_vec(&preset("embedded", 1)).unwrap()),
            )],
        );
        let error = GatePresetCatalog::from_image(&captured).unwrap_err();
        assert!(error.to_string().contains("must match embedded name"));
    }
}
