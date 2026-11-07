//! Read-only storage boundary for validation rules.
//!
//! [`load_ruleset`] reads `rules.toml` and its referenced schemas from one live
//! root. Mutation flows instead resolve bytes from a closed repository image and
//! publish them through a mutation session.

use crate::config::JitConfig;
use crate::declarations::rules::{RuleConfigError, RuleSet};
use crate::repository_state::{RepositoryLayout, VirtualPath};
use std::collections::BTreeMap;

/// Load `.jit/rules.toml` and its referenced schemas from one selected data root.
///
/// A missing rules file yields an empty declaration set. Missing or unreadable
/// required schemas are errors; optional default-rule projections are skipped.
/// `config` is explicit so item-kind expansion never reopens ambient state.
pub fn load_ruleset(
    layout: &RepositoryLayout,
    config: &JitConfig,
) -> Result<RuleSet, RuleConfigError> {
    let path = layout.resolve(&VirtualPath::RULES)?;
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RuleSet::empty());
        }
        Err(source) => return Err(RuleConfigError::Io { path, source }),
    };
    let schemas =
        RuleSet::schema_requests(&content)?.into_iter().try_fold(
            BTreeMap::new(),
            |mut schemas, request| {
                let schema_virtual_path = VirtualPath::schema_reference(&request.reference)
                    .map_err(|_| RuleConfigError::InvalidSchemaReference {
                        rule: request.rule.clone(),
                        reference: request.reference.clone(),
                        message: "reference must be one relative 'schemas/<name>.json' file"
                            .to_string(),
                    })?;
                let schema_path = layout.resolve(&schema_virtual_path)?;
                match std::fs::read(&schema_path) {
                    Ok(bytes) => {
                        schemas.insert(request.reference, bytes);
                    }
                    Err(_) if !request.required => {}
                    Err(source) => {
                        return Err(RuleConfigError::SchemaIo {
                            rule: request.rule,
                            path: schema_path,
                            source,
                        });
                    }
                }
                Ok(schemas)
            },
        )?;
    RuleSet::parse(&content, Some(config), schemas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn data_path(data_root: &Path, path: &VirtualPath) -> PathBuf {
        crate::test_utils::repository_layout_for_data_root(data_root)
            .resolve(path)
            .unwrap()
    }

    fn schema_reference(name: &str) -> String {
        VirtualPath::schema(name)
            .unwrap()
            .relative()
            .as_str()
            .to_owned()
    }

    fn empty_config() -> JitConfig {
        toml::from_str("").unwrap()
    }

    #[test]
    fn test_load_ruleset_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load_ruleset(
                &crate::test_utils::repository_layout_for_data_root(dir.path()),
                &empty_config()
            )
            .unwrap(),
            RuleSet::empty()
        );
    }

    #[test]
    fn test_load_ruleset_malformed_file_is_an_error_rather_than_an_empty_set() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            data_path(dir.path(), &VirtualPath::RULES),
            "[[rules]]\nname = \"broken\"\nassert = {}\n",
        )
        .unwrap();
        assert!(load_ruleset(
            &crate::test_utils::repository_layout_for_data_root(dir.path()),
            &empty_config()
        )
        .is_err());
    }

    #[test]
    fn test_load_ruleset_reads_referenced_schema_from_same_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(data_path(dir.path(), &VirtualPath::SCHEMAS)).unwrap();
        std::fs::write(
            data_path(dir.path(), &VirtualPath::RULES),
            format!(
                "[[rules]]\nname = \"schema\"\nassert = {{ json-schema = \"{}\" }}\n",
                schema_reference("body.json")
            ),
        )
        .unwrap();
        std::fs::write(
            data_path(dir.path(), &VirtualPath::schema("body.json").unwrap()),
            br#"{"type":"object"}"#,
        )
        .unwrap();
        assert_eq!(
            load_ruleset(
                &crate::test_utils::repository_layout_for_data_root(dir.path()),
                &empty_config()
            )
            .unwrap()
            .rules
            .len(),
            1
        );
    }

    #[test]
    fn test_load_ruleset_required_schema_missing_or_invalid_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            data_path(dir.path(), &VirtualPath::RULES),
            format!(
                "[[rules]]\nname = \"schema\"\nassert = {{ json-schema = \"{}\" }}\n",
                schema_reference("body.json")
            ),
        )
        .unwrap();
        assert!(matches!(
            load_ruleset(
                &crate::test_utils::repository_layout_for_data_root(dir.path()),
                &empty_config()
            ),
            Err(RuleConfigError::SchemaIo { .. })
        ));

        std::fs::create_dir_all(data_path(dir.path(), &VirtualPath::SCHEMAS)).unwrap();
        std::fs::write(
            data_path(dir.path(), &VirtualPath::schema("body.json").unwrap()),
            b"{not json",
        )
        .unwrap();
        assert!(matches!(
            load_ruleset(
                &crate::test_utils::repository_layout_for_data_root(dir.path()),
                &empty_config()
            ),
            Err(RuleConfigError::SchemaJson { .. })
        ));
    }

    #[test]
    fn test_load_ruleset_default_schema_failure_uses_placeholder() {
        for make_unreadable in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            if make_unreadable {
                std::fs::create_dir_all(data_path(
                    dir.path(),
                    &VirtualPath::schema("default-namespace-registry.json").unwrap(),
                ))
                .unwrap();
            }
            std::fs::write(
                data_path(dir.path(), &VirtualPath::RULES),
                format!(
                    "[[rules]]\nname = \"namespace-registry\"\norigin = \"default\"\n\
                     assert = {{ json-schema = \"{}\" }}\n",
                    schema_reference("default-namespace-registry.json")
                ),
            )
            .unwrap();
            let set = load_ruleset(
                &crate::test_utils::repository_layout_for_data_root(dir.path()),
                &empty_config(),
            )
            .unwrap();
            assert_eq!(set.rules.len(), 1);
            match &set.rules[0].assert {
                crate::declarations::rules::Assertion::JsonSchema(source) => {
                    assert_eq!(source.schema, serde_json::json!({}));
                }
                other => panic!("expected a JsonSchema placeholder, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_load_ruleset_kind_expansion_uses_explicit_config() {
        let dir = tempfile::tempdir().unwrap();
        let layout = crate::test_utils::repository_layout_for_data_root(dir.path());
        std::fs::write(
            layout.resolve(&VirtualPath::RULES).unwrap(),
            "[[rules]]\nname = \"coverage\"\nwhen = { type = \"epic\" }\nseverity = \"error\"\n\
             assert = { label-coverage = { kind = \"requirement\", child-state = \"done\" } }\n",
        )
        .unwrap();
        let config: JitConfig = toml::from_str(&crate::test_utils::shipped_item_kind_tables(&[
            "requirement",
        ]))
        .unwrap();
        assert_eq!(load_ruleset(&layout, &config).unwrap().rules.len(), 1);
        assert!(load_ruleset(&layout, &empty_config()).is_err());
    }
}
