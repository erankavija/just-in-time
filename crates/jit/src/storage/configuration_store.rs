//! Read-only storage boundary for assembled repository configuration.

use super::repository_state_store::ConfigurationObservation;
use crate::config::JitConfig;
use crate::repository_state::RepositoryLayout;
use anyhow::{Context, Result};

/// Load and assemble one admitted set of repository declaration files.
/// Missing files contribute empty declarations; capture and assembly failures
/// retain their error types.
/// The files parse through `memo`.
pub fn load_configuration(
    layout: &RepositoryLayout,
    memo: &crate::declarations::DeclarationMemo,
) -> Result<JitConfig> {
    let observation =
        ConfigurationObservation::capture(layout.data_root(), memo).with_context(|| {
            format!(
                "Failed to read configuration at '{}'",
                layout.data_root().display()
            )
        })?;
    Ok(observation.assemble()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::ConfigurationAssemblyError;
    use crate::repository_state::VirtualPath;
    use crate::test_utils::repository_layout_for_data_root;

    #[test]
    fn test_load_configuration_absent_data_root_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let layout = repository_layout_for_data_root(&dir.path().join("absent"));
        let config = load_configuration(&layout, &Default::default()).unwrap();
        assert!(config.type_hierarchy.is_none());
        assert!(config.templates.templates.is_empty());
        assert!(config.invariants.invariants.is_empty());
        assert!(!layout.data_root().exists());
    }

    #[test]
    fn test_load_configuration_malformed_config_names_source() {
        let dir = tempfile::tempdir().unwrap();
        let layout = repository_layout_for_data_root(dir.path());
        std::fs::write(layout.resolve(&VirtualPath::CONFIG).unwrap(), "[broken").unwrap();
        let error = load_configuration(&layout, &Default::default()).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ConfigurationAssemblyError>(),
            Some(ConfigurationAssemblyError::Config(_))
        ));
        assert!(error.to_string().contains("config.toml"));
    }

    #[test]
    fn test_load_configuration_malformed_templates_names_source() {
        let dir = tempfile::tempdir().unwrap();
        let layout = repository_layout_for_data_root(dir.path());
        std::fs::write(
            layout.resolve(&VirtualPath::TEMPLATES).unwrap(),
            "[[template",
        )
        .unwrap();
        let error = load_configuration(&layout, &Default::default()).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ConfigurationAssemblyError>(),
            Some(ConfigurationAssemblyError::Templates(_))
        ));
        assert!(error.to_string().contains("templates.toml"));
    }

    #[cfg(unix)]
    #[test]
    fn test_load_configuration_unreadable_directory_names_source() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let layout = repository_layout_for_data_root(&dir.path().join("sealed"));
        std::fs::create_dir(layout.data_root()).unwrap();
        std::fs::set_permissions(layout.data_root(), std::fs::Permissions::from_mode(0o0)).unwrap();
        let result = load_configuration(&layout, &Default::default());
        std::fs::set_permissions(layout.data_root(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        let error = result.unwrap_err();
        assert!(error.chain().any(|cause| cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)));
        assert!(format!("{error:#}").contains(layout.data_root().to_str().unwrap()));
    }
}
