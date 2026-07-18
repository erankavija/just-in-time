//! Filesystem boundary for rule declarations and referenced schema bytes.

use crate::config::JitConfig;
use crate::declarations::rules::{RuleConfigError, RuleSet};
use std::collections::BTreeMap;
use std::path::Path;

/// Load `.jit/rules.toml` and referenced schemas from one selected data root.
///
/// A missing rules file yields an empty declaration set. Configuration is
/// explicit so kind-based expansion never reopens ambient repository state.
pub fn load_ruleset(jit_root: &Path, config: &JitConfig) -> Result<RuleSet, RuleConfigError> {
    let path = jit_root.join("rules.toml");
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RuleSet::empty());
        }
        Err(source) => return Err(RuleConfigError::Io { path, source }),
    };
    parse_ruleset_with_filesystem_schemas(&content, jit_root, config)
}

/// Parse captured rules while resolving schema references at this boundary.
pub fn parse_ruleset_with_filesystem_schemas(
    content: &str,
    jit_root: &Path,
    config: &JitConfig,
) -> Result<RuleSet, RuleConfigError> {
    let schemas = RuleSet::schema_requests(content)?.into_iter().try_fold(
        BTreeMap::new(),
        |mut schemas, request| {
            let path = jit_root.join(&request.reference);
            match std::fs::read(&path) {
                Ok(bytes) => {
                    schemas.insert(request.reference, bytes);
                }
                Err(_) if !request.required => {}
                Err(source) => {
                    return Err(RuleConfigError::SchemaIo {
                        rule: request.rule,
                        path,
                        source,
                    });
                }
            }
            Ok(schemas)
        },
    )?;
    RuleSet::parse(content, Some(config), schemas)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> JitConfig {
        toml::from_str::<JitConfig>("").unwrap()
    }

    #[test]
    fn test_boundary_loader_missing_rules_is_empty() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            load_ruleset(directory.path(), &config()).unwrap(),
            RuleSet::empty()
        );
    }

    #[test]
    fn test_boundary_loader_preserves_missing_and_invalid_schema_errors() {
        let directory = tempfile::tempdir().unwrap();
        let rules = r#"
[[rules]]
name = "schema"
assert = { json-schema = "schemas/body.json" }
"#;
        std::fs::write(directory.path().join("rules.toml"), rules).unwrap();
        assert!(matches!(
            load_ruleset(directory.path(), &config()),
            Err(RuleConfigError::SchemaIo { .. })
        ));

        std::fs::create_dir_all(directory.path().join("schemas")).unwrap();
        std::fs::write(directory.path().join("schemas/body.json"), b"{not json").unwrap();
        assert!(matches!(
            load_ruleset(directory.path(), &config()),
            Err(RuleConfigError::SchemaJson { .. })
        ));
    }

    #[test]
    fn test_boundary_loader_reads_rules_and_schema_bytes() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("schemas")).unwrap();
        std::fs::write(
            directory.path().join("rules.toml"),
            r#"
[[rules]]
name = "schema"
assert = { json-schema = "schemas/body.json" }
"#,
        )
        .unwrap();
        std::fs::write(
            directory.path().join("schemas/body.json"),
            br#"{"type":"object"}"#,
        )
        .unwrap();

        let rules = load_ruleset(directory.path(), &config()).unwrap();
        assert_eq!(rules.rules.len(), 1);
    }
}
