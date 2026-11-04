mod decision_kind_tests;
mod decision_risk_story_tests;
mod derived_state_repair_tests;
mod invariant_check_cli_tests;
mod invariant_registry_story_tests;
mod item_cli_tests;
mod markdown_kind_source_path_tests;
mod project_render_cli_tests;
mod registry_json_tests;
mod risk_kind_tests;
mod type_hierarchy_fix_tests;
mod validate_cli_rule_tests;
mod validate_document_tests;
mod validate_drift_builtin_tests;
mod validate_read_tests;
mod validation_lease_tests;

/// The package carrying the generic vocabulary these suites address: the type
/// hierarchy, the label namespaces, and the addressable item kinds an item is
/// resolved against.
pub(crate) const DEFAULT_PACKAGE: &str = "jit-default";

/// The shared taxonomy configuration plus the shipped declarations of the named
/// item kinds.
pub(crate) fn config_with_shipped_kinds(kinds: &[&str]) -> String {
    format!(
        "{}\n{}",
        jit::test_taxonomy::test_taxonomy().config_fragment(),
        jit::test_utils::shipped_item_kind_tables(kinds)
    )
}
