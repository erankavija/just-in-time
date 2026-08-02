//! `cli_query_graph` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

use std::ops::Deref;
use tempfile::TempDir;

#[path = "../common/harness.rs"]
mod harness;

mod batch_export_cli_tests;
mod check_links_tests;
mod dep_add_redundancy_cli_tests;
mod dependency_display_tests;
mod graph_depth_tests;
mod graph_json_tests;
mod hierarchy_tree_tests;
mod label_constraints_tests;
mod label_filter_repeatable_query_tests;
mod label_hierarchy_e2e_test;
mod label_query_json_tests;
mod query_json_tests;
mod query_tests;
mod remote_document_tls_tests;
mod scope_validation_tests;
mod search_tests;

/// A repository initialized from the shared test taxonomy, together with the
/// declaration that authored its vocabulary.
pub(crate) struct TaxonomyRepo {
    pub(crate) temp: TempDir,
    pub(crate) taxonomy: jit::test_taxonomy::TestTaxonomy,
}

impl Deref for TaxonomyRepo {
    type Target = TempDir;

    fn deref(&self) -> &Self::Target {
        &self.temp
    }
}

/// Build a subprocess-test repository from the one shared vocabulary fixture.
pub(crate) fn setup_test_repo_with_taxonomy() -> TaxonomyRepo {
    let (temp, _storage, taxonomy) = jit::test_utils::setup_test_repo_with_taxonomy()
        .expect("shared taxonomy repository setup succeeds");
    TaxonomyRepo { temp, taxonomy }
}

pub(crate) fn type_label(taxonomy: &jit::test_taxonomy::TestTaxonomy, level: u8) -> String {
    format!("type:{}", taxonomy.type_at_level(level))
}

pub(crate) fn membership_label(
    taxonomy: &jit::test_taxonomy::TestTaxonomy,
    level: u8,
    value: &str,
) -> String {
    format!("{}:{value}", membership_namespace(taxonomy, level))
}

pub(crate) fn membership_namespace(taxonomy: &jit::test_taxonomy::TestTaxonomy, level: u8) -> &str {
    let type_name = taxonomy.type_at_level(level);
    taxonomy
        .label_associations
        .get(type_name)
        .map(String::as_str)
        .expect("every taxonomy container type has a membership namespace")
}

fn fixture_issue(title: String, description: String) -> jit::domain::Issue {
    let mut issue = jit::domain::Issue::draft(title, description);
    issue.id = uuid::Uuid::new_v4().to_string();
    issue
}
