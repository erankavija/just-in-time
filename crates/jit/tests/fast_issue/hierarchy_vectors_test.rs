//! REQ-03 test-vector conformance.
//!
//! Loads the fixture `test-vectors/hierarchy_resolution.json` and asserts the
//! core resolver [`jit::graph::hierarchy::resolve_hierarchy`] reproduces the
//! committed `expected` output. `resolve_hierarchy` is the sole implementation
//! of the canonical DAG-authoritative resolution; every consumer (CLI output,
//! graph export, the web server's `/graph` payload) projects its result, so
//! this fixture pins the rule for all of them. Its `r-*` nodes pin the
//! canonicalization: an epic's redundant direct edge to a task its story already
//! reaches leaves the task under the story.

use std::collections::HashMap;

use jit::domain::type_taxonomy::HierarchyConfig;
use jit::graph::hierarchy::{resolve_hierarchy, HierarchyNode, NodeHierarchy};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    hierarchy: HashMap<String, u8>,
    nodes: Vec<FixtureNode>,
    expected: HashMap<String, NodeHierarchy<String>>,
}

#[derive(Deserialize)]
struct FixtureNode {
    id: String,
    #[serde(rename = "type")]
    type_name: Option<String>,
    dependencies: Vec<String>,
}

impl HierarchyNode for FixtureNode {
    type Id = String;

    fn id(&self) -> &String {
        &self.id
    }
    fn dependencies(&self) -> impl Iterator<Item = &String> + '_ {
        self.dependencies.iter()
    }
    fn type_name(&self) -> Option<&str> {
        self.type_name.as_deref()
    }
}

#[test]
fn test_shared_vectors_match_core_resolution() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test-vectors/hierarchy_resolution.json"
    );
    let raw =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read shared fixture {path}: {e}"));
    let fixture: Fixture = serde_json::from_str(&raw).expect("parse shared fixture");

    let config = HierarchyConfig::new(fixture.hierarchy.clone(), HashMap::new())
        .expect("fixture hierarchy is valid");
    let refs: Vec<&FixtureNode> = fixture.nodes.iter().collect();
    let resolution = resolve_hierarchy(&refs, &config);

    assert_eq!(
        resolution.len(),
        fixture.expected.len(),
        "resolved node count must match the fixture"
    );

    for (id, expected) in &fixture.expected {
        let facts = resolution
            .get(id)
            .unwrap_or_else(|| panic!("missing node {id}"));
        assert_eq!(facts, expected, "resolution mismatch for node {id}");
    }
}
