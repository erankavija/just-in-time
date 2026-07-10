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

use jit::graph::hierarchy::{resolve_hierarchy, HierarchyNode};
use jit::type_hierarchy::HierarchyConfig;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    hierarchy: HashMap<String, u8>,
    nodes: Vec<FixtureNode>,
    expected: HashMap<String, ExpectedNode>,
}

#[derive(Deserialize)]
struct FixtureNode {
    id: String,
    #[serde(rename = "type")]
    type_name: Option<String>,
    dependencies: Vec<String>,
}

#[derive(Deserialize, PartialEq, Debug)]
struct ExpectedNode {
    parent: Option<String>,
    children: Vec<String>,
    cluster: Option<String>,
    rank: u32,
}

impl HierarchyNode for FixtureNode {
    fn id(&self) -> &str {
        &self.id
    }
    fn dependencies(&self) -> &[String] {
        &self.dependencies
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
        let actual = ExpectedNode {
            parent: facts.parent.clone(),
            children: facts.children.clone(),
            cluster: facts.cluster.clone(),
            rank: facts.rank,
        };
        assert_eq!(&actual, expected, "resolution mismatch for node {id}");
    }
}
