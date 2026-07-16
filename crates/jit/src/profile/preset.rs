use super::{Contribution, EmbeddedProfilePackage, ProjectedFileMode};
use serde_json::Value;
use std::collections::BTreeSet;

/// Gate definitions and ordinary assets derived from one package.
#[derive(Debug, Clone, PartialEq)]
pub struct PresetProjection {
    /// Gate tables in manifest order.
    pub gates: Vec<Value>,
    /// Asset target/mode pairs in manifest order.
    pub assets: Vec<(String, ProjectedFileMode)>,
}

/// Independently maintained preset inventory used for equivalence checks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetInventory {
    /// Expected gate keys.
    pub gate_keys: Vec<String>,
    /// Expected asset targets.
    pub asset_targets: Vec<String>,
}

/// Kind of mismatch in an independent preset inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PresetInventoryKind {
    /// Package-derived item is absent from the independent inventory.
    Missing,
    /// Independent inventory declares an item absent from the package.
    Extra,
}

/// One deterministic preset-inventory mismatch.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PresetInventoryFinding {
    /// `gate:<key>` or `asset:<target>`.
    pub identity: String,
    /// Mismatch classification.
    pub kind: PresetInventoryKind,
}

/// Derive preset-compatible gate definitions and assets from package data.
///
/// This is intentionally a projection, not a second authored inventory.
pub fn derive_preset_projection(package: &EmbeddedProfilePackage<'_>) -> PresetProjection {
    let gates = package
        .manifest()
        .contributions
        .iter()
        .filter_map(|contribution| match contribution {
            Contribution::KeyedArray {
                target: super::KeyedArrayTarget::Gates,
                value,
                ..
            } => Some(value.clone()),
            _ => None,
        })
        .collect();
    let assets = package
        .manifest()
        .assets
        .iter()
        .map(|asset| {
            (
                asset.target.clone(),
                if asset.executable {
                    ProjectedFileMode::Executable
                } else {
                    ProjectedFileMode::Regular
                },
            )
        })
        .collect();
    PresetProjection { gates, assets }
}

/// Compare an independent preset inventory to the package-derived projection.
pub fn compare_preset_inventory(
    projection: &PresetProjection,
    inventory: &PresetInventory,
) -> Vec<PresetInventoryFinding> {
    let derived = projection
        .gates
        .iter()
        .filter_map(|gate| gate.get("key").and_then(Value::as_str))
        .map(|key| format!("gate:{key}"))
        .chain(
            projection
                .assets
                .iter()
                .map(|(target, _)| format!("asset:{target}")),
        )
        .collect::<BTreeSet<_>>();
    let independent = inventory
        .gate_keys
        .iter()
        .map(|key| format!("gate:{key}"))
        .chain(
            inventory
                .asset_targets
                .iter()
                .map(|target| format!("asset:{target}")),
        )
        .collect::<BTreeSet<_>>();

    let mut findings = derived
        .difference(&independent)
        .map(|identity| PresetInventoryFinding {
            identity: identity.clone(),
            kind: PresetInventoryKind::Missing,
        })
        .chain(
            independent
                .difference(&derived)
                .map(|identity| PresetInventoryFinding {
                    identity: identity.clone(),
                    kind: PresetInventoryKind::Extra,
                }),
        )
        .collect::<Vec<_>>();
    findings.sort();
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use include_dir::{include_dir, Dir};

    static PACKAGE: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/synthetic-valid");

    #[test]
    fn test_preset_projection_derives_package_gates_and_assets() {
        let package = EmbeddedProfilePackage::from_dir(&PACKAGE).unwrap();
        let projection = derive_preset_projection(&package);
        assert_eq!(projection.gates.len(), 1);
        assert_eq!(projection.gates[0]["key"], "synthetic-review");
        assert_eq!(
            projection.assets,
            vec![
                ("docs/workflow.txt".to_string(), ProjectedFileMode::Regular),
                ("bin/check.sh".to_string(), ProjectedFileMode::Executable),
            ]
        );
    }

    #[test]
    fn test_independent_preset_inventory_mismatch_is_detected() {
        let package = EmbeddedProfilePackage::from_dir(&PACKAGE).unwrap();
        let projection = derive_preset_projection(&package);
        let findings = compare_preset_inventory(
            &projection,
            &PresetInventory {
                gate_keys: vec!["independent-review".to_string()],
                asset_targets: vec!["docs/workflow.txt".to_string()],
            },
        );
        assert_eq!(
            findings,
            vec![
                PresetInventoryFinding {
                    identity: "asset:bin/check.sh".to_string(),
                    kind: PresetInventoryKind::Missing,
                },
                PresetInventoryFinding {
                    identity: "gate:independent-review".to_string(),
                    kind: PresetInventoryKind::Extra,
                },
                PresetInventoryFinding {
                    identity: "gate:synthetic-review".to_string(),
                    kind: PresetInventoryKind::Missing,
                },
            ]
        );
    }
}
