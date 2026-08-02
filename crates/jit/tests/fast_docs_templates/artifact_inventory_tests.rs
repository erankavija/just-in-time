use jit::domain::artifact_inventory::{
    inventory_explicit_roots, pinned_root_requests, ExplicitRootTarget, PinnedRootEvidence,
    PinnedRootEvidenceMap,
};
use jit::domain::artifact_plan::{ArtifactVersion, BlockerCode, EvidenceCode, WarningCode};
use jit::domain::{DocumentReference, Issue, State};
use jit::test_taxonomy::{test_taxonomy, TestTaxonomy};

const OID_A: &str = "0123456789abcdef0123456789abcdef01234567";
const OID_B: &str = "fedcba9876543210fedcba9876543210fedcba98";

#[derive(Default)]
struct FakePinnedResolver {
    resolutions: PinnedRootEvidenceMap,
}

impl FakePinnedResolver {
    fn resolves(mut self, revision: &str, path: &str, oid: &str) -> Self {
        self.resolutions.insert(
            (revision.to_string(), path.to_string()),
            PinnedRootEvidence::Resolved(ArtifactVersion::pinned(oid).unwrap()),
        );
        self
    }

    fn fails(mut self, revision: &str, path: &str) -> Self {
        self.resolutions.insert(
            (revision.to_string(), path.to_string()),
            PinnedRootEvidence::Unavailable,
        );
        self
    }
}

impl std::ops::Deref for FakePinnedResolver {
    type Target = PinnedRootEvidenceMap;
    fn deref(&self) -> &Self::Target {
        &self.resolutions
    }
}

fn issue(id: &str, taxonomy: &TestTaxonomy, level: u8, state: State) -> Issue {
    let mut issue = crate::fixture_issue(id.to_string(), String::new());
    issue.id = id.to_string();
    issue.labels = vec![format!("type:{}", taxonomy.type_at_level(level))];
    issue.state = state;
    issue
}

fn document(path: &str, commit: Option<&str>) -> DocumentReference {
    let mut document = DocumentReference::new(path.to_string());
    document.commit = commit.map(str::to_string);
    document
}

#[test]
fn test_container_inventory_uses_resolved_children_not_raw_dependency_closure() {
    let taxonomy = test_taxonomy();
    let mut selected = issue("z-selected", &taxonomy, 2, State::Done);
    let mut other = issue("a-other", &taxonomy, 2, State::Done);
    let mut local = issue("local-child", &taxonomy, 4, State::Done);
    let mut sequenced_elsewhere = issue("outside-child", &taxonomy, 4, State::InProgress);

    selected.dependencies = vec![local.id.clone(), sequenced_elsewhere.id.clone()];
    other.dependencies = vec![sequenced_elsewhere.id.clone()];
    local.documents = vec![document("./reports//shared.csv", None)];
    sequenced_elsewhere.documents = vec![
        document("reports/shared.csv", None),
        document("reports/outside-only.png", None),
    ];

    let issues = vec![sequenced_elsewhere, local, other, selected];
    let inventory = inventory_explicit_roots(
        &issues,
        &taxonomy.hierarchy_config(),
        ExplicitRootTarget::Container("z-selected"),
        &FakePinnedResolver::default(),
    )
    .unwrap();
    let mut reversed = issues.clone();
    reversed.reverse();
    let reordered = inventory_explicit_roots(
        &reversed,
        &taxonomy.hierarchy_config(),
        ExplicitRootTarget::Container("z-selected"),
        &FakePinnedResolver::default(),
    )
    .unwrap();

    assert_eq!(inventory, reordered);
    assert_eq!(inventory.member_ids(), ["local-child", "z-selected"]);
    assert_eq!(inventory.artifacts().len(), 1);
    let artifact = &inventory.artifacts()[0];
    assert_eq!(artifact.source(), "reports/shared.csv");
    assert_eq!(artifact.version(), &ArtifactVersion::WorkingTree);
    assert_eq!(artifact.owners().len(), 2);
    assert_eq!(artifact.owners()[0].issue, "local-child");
    assert!(artifact.owners()[0].inside_subtree);
    assert_eq!(artifact.owners()[0].document_index, 0);
    assert_eq!(artifact.owners()[1].issue, "outside-child");
    assert!(!artifact.owners()[1].inside_subtree);
    assert_eq!(artifact.owners()[1].state, State::InProgress);
}

#[test]
fn test_container_inventory_includes_opaque_roots_without_adapters() {
    let taxonomy = test_taxonomy();
    let mut container = issue("container", &taxonomy, 2, State::Done);
    let mut child = issue("child", &taxonomy, 4, State::Done);
    container.dependencies = vec![child.id.clone()];
    child.documents = vec![
        document("data/table.csv", None),
        document("figures/pixels.png", None),
        document("figures/vector.svg", None),
    ];

    let inventory = inventory_explicit_roots(
        &[container, child],
        &taxonomy.hierarchy_config(),
        ExplicitRootTarget::Container("container"),
        &FakePinnedResolver::default(),
    )
    .unwrap();

    assert_eq!(
        inventory
            .artifacts()
            .iter()
            .map(|artifact| artifact.source())
            .collect::<Vec<_>>(),
        ["data/table.csv", "figures/pixels.png", "figures/vector.svg"]
    );
    assert!(inventory
        .artifacts()
        .iter()
        .all(|artifact| artifact.format().is_none()));
}

#[test]
fn test_document_inventory_normalizes_zero_owner_opaque_root() {
    let taxonomy = test_taxonomy();
    let inventory = inventory_explicit_roots(
        &[],
        &taxonomy.hierarchy_config(),
        ExplicitRootTarget::Document("./exports/../figures//chart.svg"),
        &FakePinnedResolver::default(),
    )
    .unwrap();

    assert!(inventory.member_ids().is_empty());
    assert_eq!(inventory.artifacts().len(), 1);
    let artifact = &inventory.artifacts()[0];
    assert_eq!(artifact.source(), "figures/chart.svg");
    assert_eq!(artifact.version(), &ArtifactVersion::WorkingTree);
    assert!(artifact.owners().is_empty());
    assert_eq!(artifact.warnings().len(), 1);
    assert_eq!(artifact.warnings()[0].code, WarningCode::NoOwner);
}

#[test]
fn test_mixed_pinned_and_unpinned_owners_group_by_canonical_version() {
    let taxonomy = test_taxonomy();
    let mut z_owner = issue("z-owner", &taxonomy, 4, State::Done);
    z_owner.documents = vec![
        document("docs/report.csv", Some("release-v1")),
        document("docs/report.csv", None),
        document("docs/report.csv", Some("release-v2")),
    ];
    let mut a_owner = issue("a-owner", &taxonomy, 4, State::Rejected);
    a_owner.documents = vec![
        document("./docs/report.csv", Some("01234567")),
        document("docs/other.csv", None),
    ];
    let resolver = FakePinnedResolver::default()
        .resolves("release-v1", "docs/report.csv", OID_A)
        .resolves("01234567", "docs/report.csv", OID_A)
        .resolves("release-v2", "docs/report.csv", OID_B);

    let inventory = inventory_explicit_roots(
        &[z_owner, a_owner],
        &taxonomy.hierarchy_config(),
        ExplicitRootTarget::Document("docs/report.csv"),
        &resolver,
    )
    .unwrap();

    assert_eq!(inventory.artifacts().len(), 3);
    let versions = inventory
        .artifacts()
        .iter()
        .map(|artifact| artifact.version().as_str())
        .collect::<Vec<_>>();
    assert_eq!(versions, [OID_A, OID_B, "working-tree"]);

    let first_pin = &inventory.artifacts()[0];
    assert_eq!(first_pin.owners().len(), 2);
    assert_eq!(first_pin.evidence(), [EvidenceCode::PinnedHistorical]);
    assert!(first_pin.destination().is_none());
    assert_eq!(first_pin.owners()[0].issue, "a-owner");
    assert_eq!(first_pin.owners()[0].document_index, 0);
    assert!(first_pin.owners()[0].pinned);
    assert_eq!(first_pin.owners()[1].issue, "z-owner");
    assert_eq!(first_pin.owners()[1].document_index, 0);

    let working_tree = &inventory.artifacts()[2];
    assert_eq!(working_tree.owners().len(), 1);
    assert_eq!(working_tree.owners()[0].issue, "z-owner");
    assert_eq!(working_tree.owners()[0].document_index, 1);
    assert!(!working_tree.owners()[0].pinned);
}

#[test]
fn test_pinned_resolution_or_read_failure_blocks_without_working_tree_owner_fallback() {
    let taxonomy = test_taxonomy();
    let mut issue = issue("owner", &taxonomy, 4, State::Done);
    issue.documents = vec![document("docs/history.md", Some("missing-tag"))];
    let resolver = FakePinnedResolver::default().fails("missing-tag", "docs/history.md");

    let inventory = inventory_explicit_roots(
        &[issue],
        &taxonomy.hierarchy_config(),
        ExplicitRootTarget::Document("docs/history.md"),
        &resolver,
    )
    .unwrap();

    assert_eq!(inventory.artifacts().len(), 1);
    assert_eq!(
        inventory.artifacts()[0].version(),
        &ArtifactVersion::WorkingTree
    );
    assert!(inventory.artifacts()[0].owners().is_empty());
    assert_eq!(inventory.blockers().len(), 1);
    assert_eq!(inventory.blockers()[0].code, BlockerCode::PinnedReadFailed);
    assert_eq!(
        inventory.blockers()[0].path.as_deref(),
        Some("docs/history.md")
    );
}

#[test]
fn test_pinned_root_requests_are_target_relevant_and_deduplicated() {
    let taxonomy = test_taxonomy();
    let mut container = issue("selected", &taxonomy, 2, State::Done);
    let mut child = issue("child", &taxonomy, 4, State::Done);
    let mut outside = issue("outside", &taxonomy, 4, State::Done);
    container.dependencies = vec![child.id.clone()];
    child.documents = vec![
        document("docs/shared.md", Some("release")),
        document("./docs/shared.md", Some("release")),
    ];
    outside.documents = vec![document("docs/outside.md", Some("other"))];
    let issues = [outside, child, container];
    let requests = |target| {
        pinned_root_requests(&issues, &taxonomy.hierarchy_config(), target)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        requests(ExplicitRootTarget::Container("selected")),
        [("release".into(), "docs/shared.md".into())]
    );
    assert_eq!(
        requests(ExplicitRootTarget::Document("docs/outside.md")),
        [("other".into(), "docs/outside.md".into())]
    );
}
