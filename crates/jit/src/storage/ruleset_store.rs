//! Read-only storage boundary for validation rules.
//!
//! [`load_ruleset`] reads `rules.toml` and its referenced schemas from one live
//! root. Mutation flows instead resolve bytes from a closed repository image and
//! publish them through the repository-state transaction boundary.

use crate::config::JitConfig;
use crate::declarations::rules::{RuleConfigError, RuleSet};
use std::collections::BTreeMap;
use std::path::Path;

const RULES_FILE: &str = "rules.toml";

/// Load `.jit/rules.toml` and its referenced schemas from one selected data root.
///
/// A missing rules file yields an empty declaration set. Missing or unreadable
/// required schemas are errors; optional default-rule projections are skipped.
/// `config` is explicit so item-kind expansion never reopens ambient state.
pub fn load_ruleset(jit_root: &Path, config: &JitConfig) -> Result<RuleSet, RuleConfigError> {
    let path = jit_root.join(RULES_FILE);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RuleSet::empty());
        }
        Err(source) => return Err(RuleConfigError::Io { path, source }),
    };
    let schemas = RuleSet::schema_requests(&content)?.into_iter().try_fold(
        BTreeMap::new(),
        |mut schemas, request| {
            let schema_path = jit_root.join(&request.reference);
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

    fn empty_config() -> JitConfig {
        toml::from_str("").unwrap()
    }

    #[test]
    fn test_load_ruleset_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load_ruleset(dir.path(), &empty_config()).unwrap(),
            RuleSet::empty()
        );
    }

    #[test]
    fn test_load_ruleset_reads_referenced_schema_from_same_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("schemas")).unwrap();
        std::fs::write(
            dir.path().join(RULES_FILE),
            "[[rules]]\nname = \"schema\"\nassert = { json-schema = \"schemas/body.json\" }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("schemas/body.json"),
            br#"{"type":"object"}"#,
        )
        .unwrap();
        assert_eq!(
            load_ruleset(dir.path(), &empty_config())
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
            dir.path().join(RULES_FILE),
            "[[rules]]\nname = \"schema\"\nassert = { json-schema = \"schemas/body.json\" }\n",
        )
        .unwrap();
        assert!(matches!(
            load_ruleset(dir.path(), &empty_config()),
            Err(RuleConfigError::SchemaIo { .. })
        ));

        std::fs::create_dir_all(dir.path().join("schemas")).unwrap();
        std::fs::write(dir.path().join("schemas/body.json"), b"{not json").unwrap();
        assert!(matches!(
            load_ruleset(dir.path(), &empty_config()),
            Err(RuleConfigError::SchemaJson { .. })
        ));
    }

    #[test]
    fn test_load_ruleset_default_schema_failure_uses_placeholder() {
        for make_unreadable in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            if make_unreadable {
                std::fs::create_dir_all(dir.path().join("schemas/default-namespace-registry.json"))
                    .unwrap();
            }
            std::fs::write(
                dir.path().join(RULES_FILE),
                "[[rules]]\nname = \"namespace-registry\"\norigin = \"default\"\n\
                 assert = { json-schema = \"schemas/default-namespace-registry.json\" }\n",
            )
            .unwrap();
            let set = load_ruleset(dir.path(), &empty_config()).unwrap();
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
        std::fs::write(
            dir.path().join(RULES_FILE),
            "[[rules]]\nname = \"coverage\"\nwhen = { type = \"epic\" }\nseverity = \"error\"\n\
             assert = { label-coverage = { kind = \"requirement\", child-state = \"done\" } }\n",
        )
        .unwrap();
        let config: JitConfig = toml::from_str(
            r#"
[item_kinds.requirement]
section = "success_criteria"
id-pattern = "[A-Z][A-Z0-9]*-[0-9]+"
markers = ["[hard]"]
link-namespaces = ["satisfies"]
scope = "issue"
source-of-truth = "markdown-first"
"#,
        )
        .unwrap();
        assert_eq!(load_ruleset(dir.path(), &config).unwrap().rules.len(), 1);
        assert!(load_ruleset(dir.path(), &empty_config()).is_err());
    }
}
