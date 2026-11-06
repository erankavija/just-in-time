use jit::domain::artifact_classifier::{
    ArtifactClassificationInventory, ArtifactClassificationPolicy,
};
use jit::domain::artifact_discovery::{
    discover_archive_artifacts, expand_artifact_closure, ArtifactClosure, ArtifactClosureState,
    ArtifactEvidence, ArtifactEvidenceMap, ArtifactListingScope,
};
use jit::domain::artifact_inventory::{
    inventory_explicit_roots, ExplicitRootInventory, ExplicitRootTarget, PinnedRootEvidence,
    PinnedRootEvidenceMap,
};
use jit::domain::artifact_plan::{
    normalize_artifact_path, ArtifactVersion, BlockerCode, EdgeResolutionMode, WarningCode,
};
use jit::domain::{Issue, State};
use jit::test_utils::{document_reference_at, issue_id};
use std::collections::{BTreeMap, BTreeSet};

const OID: &str = "0123456789abcdef0123456789abcdef01234567";

/// A policy that declares no development root, so no source lies outside one
/// and discovery follows every local link it resolves.
fn unbounded_policy() -> ArtifactClassificationPolicy {
    ArtifactClassificationPolicy::configured("", vec![], vec![], "")
}

/// A policy whose development root is `dev`, so every other repository path is
/// a frontier discovery stops at.
fn development_root_policy() -> ArtifactClassificationPolicy {
    ArtifactClassificationPolicy::configured(
        "dev",
        vec!["dev/active".to_string()],
        vec![],
        "dev/archive",
    )
}

#[test]
fn test_terminal_evidence_closes_without_parsing_or_more_needs() {
    for terminal in [
        ArtifactEvidence::Missing,
        ArtifactEvidence::Symlink,
        ArtifactEvidence::Unsupported,
        ArtifactEvidence::InvalidPath,
    ] {
        let evidence = ArtifactEvidenceMap::from([("root.md".to_string(), terminal)]);
        let result = expand_artifact_closure(
            ArtifactClosureState::new(["root.md".to_string()]),
            &evidence,
        );
        assert!(matches!(result, ArtifactClosure::Complete(parsed) if parsed.is_empty()));
    }
}

/// An in-memory repository of regular files and symbolic links, answering each
/// path the closure requests with the evidence a no-follow capture reports.
#[derive(Default)]
struct Tree {
    files: BTreeMap<String, Vec<u8>>,
    symlinks: BTreeSet<String>,
}

impl Tree {
    fn write(&mut self, path: &str, content: &str) {
        self.files
            .insert(path.to_string(), content.as_bytes().to_vec());
    }

    fn symlink(&mut self, path: &str) {
        self.symlinks.insert(path.to_string());
    }

    /// A link at `path` or at any ancestor is reported as a link, never followed.
    fn evidence(&self, path: &str) -> ArtifactEvidence {
        let components = path.split('/').collect::<Vec<_>>();
        if components
            .iter()
            .any(|component| matches!(*component, "" | "." | ".."))
        {
            return ArtifactEvidence::InvalidPath;
        }
        for depth in 1..=components.len() {
            let prefix = components[..depth].join("/");
            if self.symlinks.contains(&prefix) {
                return ArtifactEvidence::Symlink;
            }
            if let Some(bytes) = self.files.get(&prefix) {
                return if depth == components.len() {
                    ArtifactEvidence::File(bytes.clone())
                } else {
                    ArtifactEvidence::Unsupported
                };
            }
        }
        let beneath = format!("{path}/");
        if self
            .files
            .keys()
            .chain(&self.symlinks)
            .any(|entry| entry.starts_with(&beneath))
        {
            ArtifactEvidence::Directory {
                scope: ArtifactListingScope::MetadataOnly,
                entries: Vec::new(),
            }
        } else {
            ArtifactEvidence::Missing
        }
    }

    /// Close `inventory` and every issue document over this tree, then derive
    /// the selected artifacts.
    fn discover_inventory(
        &self,
        inventory: ExplicitRootInventory,
        issues: &[Issue],
        policy: &ArtifactClassificationPolicy,
    ) -> ArtifactClassificationInventory {
        let roots = inventory
            .artifacts()
            .iter()
            .filter(|entry| entry.version() == &ArtifactVersion::WorkingTree)
            .map(|entry| entry.source().to_string())
            .chain(
                issues
                    .iter()
                    .flat_map(|issue| issue.documents.iter())
                    .filter(|document| document.commit.is_none())
                    .map(|document| normalize_artifact_path(&document.path)),
            )
            .collect::<BTreeSet<_>>();
        let mut evidence = ArtifactEvidenceMap::new();
        let mut state = ArtifactClosureState::new(roots);
        let parsed = loop {
            match expand_artifact_closure(state, &evidence) {
                ArtifactClosure::Complete(parsed) => break parsed,
                ArtifactClosure::Needs { paths, state: next } => {
                    paths.into_iter().for_each(|path| {
                        let fact = self.evidence(&path);
                        evidence.insert(path, fact);
                    });
                    state = next;
                }
            }
        };
        discover_archive_artifacts(inventory, issues, &evidence, &parsed, policy)
            .unwrap()
            .0
    }

    fn discover(&self, root: &str) -> ArtifactClassificationInventory {
        self.discover_with_policy(root, &unbounded_policy())
    }

    fn discover_with_policy(
        &self,
        root: &str,
        policy: &ArtifactClassificationPolicy,
    ) -> ArtifactClassificationInventory {
        let inventory = inventory_explicit_roots(
            &[],
            &jit::test_taxonomy::test_taxonomy().hierarchy_config(),
            ExplicitRootTarget::Document(root),
            &PinnedRootEvidenceMap::new(),
        )
        .unwrap();
        self.discover_inventory(inventory, &[], policy)
    }

    fn sources(inventory: &ArtifactClassificationInventory) -> Vec<&str> {
        inventory
            .artifacts()
            .iter()
            .map(|entry| entry.source())
            .collect()
    }
}

#[test]
fn test_discover_archive_artifacts_admits_an_out_of_root_artifact_but_none_of_its_descendants() {
    let mut repo = Tree::default();
    repo.write("dev/active/plan.md", "[hub](../../README.md)");
    repo.write("README.md", "[guide](docs/guide.md)");
    repo.write("docs/guide.md", "[deeper](reference/deeper.md)");
    repo.write("docs/reference/deeper.md", "deeper");

    let unbounded = repo.discover_with_policy("dev/active/plan.md", &unbounded_policy());
    assert_eq!(
        Tree::sources(&unbounded),
        [
            "README.md",
            "dev/active/plan.md",
            "docs/guide.md",
            "docs/reference/deeper.md"
        ]
    );

    let bounded = repo.discover_with_policy("dev/active/plan.md", &development_root_policy());
    assert_eq!(
        Tree::sources(&bounded),
        ["README.md", "dev/active/plan.md"],
        "no artifact reached only through the out-of-root hub may join the plan"
    );
    let hub = bounded
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "README.md")
        .unwrap();
    assert!(hub.edges().is_empty());
    let scanned = bounded
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "dev/active/plan.md")
        .unwrap();
    assert!(scanned
        .edges()
        .iter()
        .any(|edge| edge.target.as_deref() == Some("README.md")));
    assert!(bounded
        .artifacts()
        .iter()
        .all(|entry| entry.blockers().is_empty()));
    assert!(bounded.blockers.is_empty());
}

#[test]
fn test_recursive_discovery_reaches_html_css_import_url_figure_and_script_src() {
    let mut repo = Tree::default();
    repo.write(
        "slides/index.html",
        r#"<link rel="stylesheet" href="theme.css"><script src="js/app.js"></script>"#,
    );
    repo.write(
        "slides/theme.css",
        r#"@import "nested/colors.css"; .hero { background: url("figures/hero.png") }"#,
    );
    repo.write(
        "slides/nested/colors.css",
        r#"@import url('../base.css'); .icon { background: url('../figures/icon.svg#mark') }"#,
    );
    repo.write("slides/base.css", "body { color: black }");
    repo.write("slides/figures/hero.png", "hero");
    repo.write("slides/figures/icon.svg", "<svg/>");
    repo.write("slides/js/app.js", "console.log('static bundle member');");

    let discovered = repo.discover("slides/index.html");
    assert_eq!(
        discovered
            .artifacts()
            .iter()
            .map(|entry| entry.source())
            .collect::<Vec<_>>(),
        [
            "slides/base.css",
            "slides/figures/hero.png",
            "slides/figures/icon.svg",
            "slides/index.html",
            "slides/js/app.js",
            "slides/nested/colors.css",
            "slides/theme.css",
        ]
    );

    let html = discovered
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "slides/index.html")
        .unwrap();
    assert!(html.edges().iter().any(|edge| {
        edge.reference == "theme.css"
            && edge.target.as_deref() == Some("slides/theme.css")
            && edge.resolution_mode == EdgeResolutionMode::Relative
    }));
    assert!(html.edges().iter().any(|edge| {
        edge.reference == "js/app.js" && edge.target.as_deref() == Some("slides/js/app.js")
    }));

    let nested = discovered
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "slides/nested/colors.css")
        .unwrap();
    assert!(nested
        .edges()
        .iter()
        .any(|edge| edge.target.as_deref() == Some("slides/base.css")));
    assert!(nested
        .edges()
        .iter()
        .any(|edge| edge.target.as_deref() == Some("slides/figures/icon.svg")));
}

#[test]
fn test_markdown_links_preserve_relative_and_root_relative_edge_metadata() {
    let mut repo = Tree::default();
    repo.write(
        "notes/readme.md",
        "[Sibling](guide.md) ![Logo](/assets/logo.png) [Web](https://example.test/x)",
    );
    repo.write("notes/guide.md", "guide");
    repo.write("assets/logo.png", "logo");

    let discovered = repo.discover("notes/readme.md");
    let root = discovered
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "notes/readme.md")
        .unwrap();
    assert!(root.edges().iter().any(|edge| {
        edge.target.as_deref() == Some("notes/guide.md")
            && edge.resolution_mode == EdgeResolutionMode::Relative
    }));
    assert!(root.edges().iter().any(|edge| {
        edge.target.as_deref() == Some("assets/logo.png")
            && edge.resolution_mode == EdgeResolutionMode::RootRelative
    }));
    assert!(root
        .plan_warnings()
        .iter()
        .any(|warning| warning.code == WarningCode::ExternalEdge));
}

#[test]
fn test_incremental_closure_reuses_parsed_state_for_shared_cyclic_reachability() {
    let mut evidence = ArtifactEvidenceMap::new();
    let state = ArtifactClosureState::new(["root.md".to_string()]);
    let ArtifactClosure::Needs { paths, state } = expand_artifact_closure(state, &evidence) else {
        panic!("an uncaptured root must be requested");
    };
    assert_eq!(paths, ["root.md".to_string()].into_iter().collect());
    evidence.insert(
        "root.md".into(),
        ArtifactEvidence::File(b"[a](a.md) [b](b.md)".to_vec()),
    );
    let ArtifactClosure::Needs { paths, state } = expand_artifact_closure(state, &evidence) else {
        panic!("the shared branches must be requested");
    };
    assert_eq!(
        paths,
        ["a.md".to_string(), "b.md".to_string()]
            .into_iter()
            .collect()
    );
    assert_eq!(state.parsed_paths().collect::<Vec<_>>(), ["root.md"]);
    for path in ["a.md", "b.md"] {
        evidence.insert(
            path.into(),
            ArtifactEvidence::File(b"[shared](shared.md)".to_vec()),
        );
    }
    let ArtifactClosure::Needs { paths, state } = expand_artifact_closure(state, &evidence) else {
        panic!("the shared descendant must be requested once");
    };
    assert_eq!(paths, ["shared.md".to_string()].into_iter().collect());
    evidence.insert(
        "root.md".into(),
        ArtifactEvidence::File(b"[new](uncaptured.md)".to_vec()),
    );
    evidence.insert(
        "shared.md".into(),
        ArtifactEvidence::File(b"[cycle](a.md)".to_vec()),
    );
    let ArtifactClosure::Complete(parsed) = expand_artifact_closure(state, &evidence) else {
        panic!("completed evidence must close without reparsing the changed root bytes");
    };
    assert_eq!(
        parsed.keys().map(String::as_str).collect::<Vec<_>>(),
        ["a.md", "b.md", "root.md", "shared.md"]
    );
}

#[test]
fn test_repository_escape_blocks_and_missing_embedded_target_warns() {
    let mut repo = Tree::default();
    repo.write(
        "docs/index.html",
        r#"<img src="../../outside.png"><img src="missing.png">"#,
    );

    let discovered = repo.discover("docs/index.html");
    let root = discovered
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "docs/index.html")
        .unwrap();
    assert!(root
        .blockers()
        .iter()
        .any(|blocker| blocker.code == BlockerCode::RepositoryEscape));
    let missing = discovered
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "docs/missing.png")
        .unwrap();
    assert!(root.plan_warnings().iter().any(|warning| {
        warning.code == WarningCode::MissingEdgeTarget
            && warning.path.as_deref() == Some("docs/missing.png")
    }));
    assert!(!missing
        .plan_warnings()
        .iter()
        .any(|warning| warning.code == WarningCode::MissingEdgeTarget));
    assert_eq!(missing.destination(), None);
    assert!(root.edges().iter().any(|edge| {
        edge.reference == "missing.png"
            && edge.target.as_deref() == Some("docs/missing.png")
            && edge.resolution_mode == EdgeResolutionMode::Relative
    }));
}

#[test]
fn test_directory_link_target_is_navigation_while_regular_link_target_stays_an_artifact() {
    let mut repo = Tree::default();
    repo.write("docs/index.md", "[section](section/) [guide](guide.md)");
    repo.write("docs/guide.md", "guide");
    repo.write("docs/section/child.md", "child");

    let discovered = repo.discover("docs/index.md");

    assert_eq!(
        discovered
            .artifacts()
            .iter()
            .map(|entry| entry.source())
            .collect::<Vec<_>>(),
        ["docs/guide.md", "docs/index.md"]
    );
    let root = discovered
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "docs/index.md")
        .unwrap();
    assert!(root
        .edges()
        .iter()
        .any(|edge| edge.target.as_deref() == Some("docs/guide.md")));
    assert!(!root
        .edges()
        .iter()
        .any(|edge| edge.target.as_deref() == Some("docs/section")));
    assert!(!root
        .plan_warnings()
        .iter()
        .any(|warning| warning.code == WarningCode::UnsupportedEdgeTarget));
    assert!(discovered
        .artifacts()
        .iter()
        .all(|entry| entry.blockers().is_empty()));
    assert!(discovered.blockers.is_empty());
}

#[test]
fn test_symlinked_directory_link_target_is_inventoried_rather_than_skipped_as_navigation() {
    let mut repo = Tree::default();
    repo.write("docs/index.md", "[plain](plain/) [linked](linked/)");
    repo.write("docs/plain/child.md", "child");
    repo.write("referents/bundle/secret.md", "must not be discovered");
    repo.symlink("docs/linked");

    let discovered = repo.discover("docs/index.md");

    assert_eq!(
        discovered
            .artifacts()
            .iter()
            .map(|entry| entry.source())
            .collect::<Vec<_>>(),
        ["docs/index.md", "docs/linked"]
    );
    let root = discovered
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "docs/index.md")
        .unwrap();
    assert!(root
        .edges()
        .iter()
        .any(|edge| edge.target.as_deref() == Some("docs/linked")));
    assert!(!root
        .edges()
        .iter()
        .any(|edge| edge.target.as_deref() == Some("docs/plain")));
    assert!(discovered
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "docs/linked")
        .unwrap()
        .edges()
        .is_empty());
}

#[test]
fn test_supported_text_format_matrix_warns_for_local_loader_text() {
    let cases = [
        ("format/example.md", "Example: `fetch('./payload.json')`."),
        (
            "format/example.html",
            "<script>fetch('./payload.json')</script>",
        ),
        (
            "format/example.css",
            "/* bundled by fetch('./payload.json') at runtime */",
        ),
        ("format/example.js", "fetch('./payload.json')"),
    ];

    for (path, content) in cases {
        assert_dynamic_loading_warning(path, content);
    }
}

#[test]
fn test_opaque_format_matrix_does_not_scan_loader_text() {
    for path in [
        "format/example.txt",
        "format/example.bin",
        "format/example.png",
    ] {
        assert_no_dynamic_loading_warning(path, "fetch('./payload.json')");
    }
}

#[test]
fn test_every_binding_family_warns_without_blocking() {
    let cases = [
        ("runtime/fetch.js", "fetch('./payload.json')"),
        ("runtime/import.js", "import('./lazy.js')"),
        (
            "runtime/xhr.js",
            "const xhr = new XMLHttpRequest(); xhr.open('GET', './payload.json')",
        ),
        ("runtime/worker.js", "new Worker('./worker.js')"),
        ("runtime/import-scripts.js", "importScripts('./one.js')"),
        ("runtime/static-import.js", "import value from './value.js'"),
        ("runtime/static-side-effect.js", "import './register.js'"),
        (
            "runtime/static-root-side-effect.js",
            "import '/register.js'",
        ),
        (
            "runtime/static-export.js",
            "export { value } from './value.js'",
        ),
        ("runtime/require.js", "const value = require('./value.js')"),
        (
            "runtime/data.html",
            r#"<div data-source="./payload.json"></div>"#,
        ),
    ];

    for (path, content) in cases {
        assert_dynamic_loading_warning(path, content);
    }
}

#[test]
fn test_url_resolving_loaders_warn_for_bare_same_directory_urls() {
    let cases = [
        ("runtime/bare-fetch.js", "fetch('payload.json')"),
        ("runtime/bare-fetch-route.js", "fetch('api/v1')"),
        (
            "runtime/bare-import-scripts.js",
            "importScripts('bootstrap.js')",
        ),
        ("runtime/bare-worker.js", "new Worker('worker.js')"),
        ("runtime/bare-worker-route.js", "new Worker('workers/main')"),
    ];

    for (path, content) in cases {
        assert_dynamic_loading_warning(path, content);
    }
}

#[test]
fn test_loader_external_and_non_path_literals_do_not_warn() {
    let cases = [
        ("runtime/http.js", "fetch('https://example.test/data.json')"),
        (
            "runtime/protocol-relative.js",
            "fetch('//cdn.test/data.json')",
        ),
        (
            "runtime/data-uri.js",
            "fetch('data:application/json,%7B%7D')",
        ),
        ("runtime/fragment.js", "fetch('#payload')"),
        ("runtime/query.js", "fetch('?payload=1')"),
        ("runtime/variable.js", "fetch(payloadUrl)"),
        ("runtime/template.js", "fetch(`${base}/payload.json`)"),
        (
            "runtime/external-worker.js",
            "new Worker('https://example.test/worker.js')",
        ),
        ("runtime/package-import.js", "import('package-name')"),
        ("runtime/package-require.js", "require('package-name')"),
        ("runtime/package-static.js", "import 'package-name'"),
    ];

    for (path, content) in cases {
        assert_no_dynamic_loading_warning(path, content);
    }
}

#[test]
fn test_data_attribute_quote_and_local_path_matrix_warns() {
    let quote_modes = [("double", "\""), ("single", "'"), ("unquoted", "")];
    let path_modes = [
        ("dot-relative", "./payload.json"),
        ("parent-relative", "../payload.json"),
        ("root-relative", "/payload.json"),
        ("bare-file", "payload.json"),
        ("bare-nested", "assets/payload"),
    ];

    for (quote_name, quote) in quote_modes {
        for (path_name, path) in path_modes {
            let artifact_path = format!("data/{quote_name}-{path_name}.html");
            let content = format!("<div data-source={quote}{path}{quote}></div>");
            assert_dynamic_loading_warning(&artifact_path, &content);
        }
    }
}

#[test]
fn test_data_attribute_external_url_matrix_does_not_warn() {
    let quote_modes = [("double", "\""), ("single", "'"), ("unquoted", "")];
    let external_values = [
        ("absolute", "https://example.test/payload.json"),
        ("protocol-relative", "//example.test/payload.json"),
        ("data-uri", "data:application/json,%7B%7D"),
    ];

    for (quote_name, quote) in quote_modes {
        for (external_name, value) in external_values {
            let artifact_path = format!("data/{quote_name}-{external_name}.html");
            let content = format!("<div data-source={quote}{value}{quote}></div>");
            assert_no_dynamic_loading_warning(&artifact_path, &content);
        }
    }
}

#[test]
fn test_data_attribute_non_path_bare_values_do_not_warn() {
    for content in [
        "<div data-mode=dark></div>",
        "<div data-enabled=true></div>",
        "<div data-target=#dialog></div>",
        "<div data-query='?mode=compact'></div>",
    ] {
        assert_no_dynamic_loading_warning("data/non-path.html", content);
    }
}

#[test]
fn test_markdown_code_and_prose_dynamic_loading_patterns_warn_textually() {
    let cases = [
        (
            "notes/code-example.md",
            "```js\nconst payload = fetch('./payload.json');\n```",
        ),
        (
            "notes/prose-example.md",
            "For local setup, use `import './register.js'` before startup.",
        ),
    ];

    for (path, content) in cases {
        assert_dynamic_loading_warning(path, content);
    }
}

fn assert_dynamic_loading_warning(path: &str, content: &str) {
    let mut repo = Tree::default();
    repo.write(path, content);
    let discovered = repo.discover(path);
    let entry = &discovered.artifacts[0];
    assert!(
        entry
            .plan_warnings()
            .iter()
            .any(|warning| warning.code == WarningCode::DynamicLoadingSuspected),
        "missing warning for {path}: {content}"
    );
    assert!(entry.blockers().is_empty(), "pattern blocked {path}");
    assert_eq!(
        discovered.artifacts.len(),
        1,
        "guessed a target for {path}: {content}"
    );
}

fn assert_no_dynamic_loading_warning(path: &str, content: &str) {
    let mut repo = Tree::default();
    repo.write(path, content);
    let discovered = repo.discover(path);
    let entry = &discovered.artifacts[0];
    assert!(
        !entry
            .plan_warnings()
            .iter()
            .any(|warning| warning.code == WarningCode::DynamicLoadingSuspected),
        "unexpected warning for {path}: {content}"
    );
    assert!(entry.blockers().is_empty(), "pattern blocked {path}");
    assert_eq!(
        discovered.artifacts.len(),
        1,
        "guessed a target for {path}: {content}"
    );
}

fn pin_evidence(readable: bool) -> PinnedRootEvidenceMap {
    [(
        ("release-v1".to_string(), "docs/history.html".to_string()),
        if readable {
            PinnedRootEvidence::Resolved(ArtifactVersion::pinned(OID).unwrap())
        } else {
            PinnedRootEvidence::Unavailable
        },
    )]
    .into_iter()
    .collect()
}

fn pinned_issue() -> Issue {
    let mut issue = crate::fixture_issue("history".to_string(), String::new());
    issue.id = issue_id(1);
    issue.state = State::Done;
    issue.labels = vec!["type:initiative".to_string()];
    issue.documents = vec![document_reference_at("docs/history.html", "release-v1")];
    issue
}

#[test]
fn test_readable_pinned_root_is_historical_and_never_scanned_or_constrains_working_tree() {
    let mut repo = Tree::default();
    repo.write("docs/history.html", r#"<img src="working-tree-bait.png">"#);
    repo.write("docs/working-tree-bait.png", "bait");
    let issues = [pinned_issue()];
    let inventory = inventory_explicit_roots(
        &issues,
        &jit::test_taxonomy::test_taxonomy().hierarchy_config(),
        ExplicitRootTarget::Container(&issues[0].id),
        &pin_evidence(true),
    )
    .unwrap();

    let discovered = repo.discover_inventory(inventory, &issues, &unbounded_policy());
    assert_eq!(discovered.artifacts.len(), 1);
    let historical = &discovered.artifacts[0];
    assert!(historical.version().is_pinned());
    assert!(historical.edges().is_empty());
    assert!(historical.content_identity().is_none());
    assert!(discovered.blockers.is_empty());
}

#[test]
fn test_failed_pinned_read_stays_pinned_read_failed_without_working_tree_fallback() {
    let mut repo = Tree::default();
    repo.write("docs/history.html", "working-tree fallback bait");
    let issues = [pinned_issue()];
    let inventory = inventory_explicit_roots(
        &issues,
        &jit::test_taxonomy::test_taxonomy().hierarchy_config(),
        ExplicitRootTarget::Container(&issues[0].id),
        &pin_evidence(false),
    )
    .unwrap();

    let discovered = repo.discover_inventory(inventory, &issues, &unbounded_policy());
    assert!(discovered.artifacts.is_empty());
    assert_eq!(discovered.blockers.len(), 1);
    assert_eq!(discovered.blockers[0].code, BlockerCode::PinnedReadFailed);
}

#[test]
fn test_symlinked_source_root_is_retained_without_parsing_referent_edges() {
    let mut repo = Tree::default();
    repo.write(
        "referents/root.md",
        "[referent-only child](secret/child.html)",
    );
    repo.write(
        "referents/secret/child.html",
        "<p>must not be discovered</p>",
    );
    repo.symlink("docs/root.md");

    let discovered = repo.discover("docs/root.md");

    assert_eq!(
        discovered
            .artifacts()
            .iter()
            .map(|entry| entry.source())
            .collect::<Vec<_>>(),
        ["docs/root.md"]
    );
    assert!(discovered.artifacts[0].edges().is_empty());
}

#[test]
fn test_source_beneath_symlinked_directory_is_not_parsed_or_traversed() {
    let mut repo = Tree::default();
    repo.write(
        "referents/bundle/root.md",
        "[referent-only child](secret.html)",
    );
    repo.write(
        "referents/bundle/secret.html",
        "<p>must not be discovered</p>",
    );
    repo.symlink("docs/bundle");

    let discovered = repo.discover("docs/bundle/root.md");

    assert_eq!(
        discovered
            .artifacts()
            .iter()
            .map(|entry| entry.source())
            .collect::<Vec<_>>(),
        ["docs/bundle/root.md"]
    );
    assert!(discovered.artifacts[0].edges().is_empty());
}

#[test]
fn test_symlinked_embedded_artifact_is_not_traversed_into_referent_closure() {
    let mut repo = Tree::default();
    repo.write("docs/root.html", r#"<link href="theme.css">"#);
    repo.write(
        "referents/theme.css",
        r#"@import "secret.css"; body { background: url("secret.png") }"#,
    );
    repo.write("referents/secret.css", "a { color: red }");
    repo.write("referents/secret.png", "must not be discovered");
    repo.symlink("docs/theme.css");

    let discovered = repo.discover("docs/root.html");

    assert_eq!(
        discovered
            .artifacts()
            .iter()
            .map(|entry| entry.source())
            .collect::<Vec<_>>(),
        ["docs/root.html", "docs/theme.css"]
    );
    let symlink = discovered
        .artifacts()
        .iter()
        .find(|entry| entry.source() == "docs/theme.css")
        .unwrap();
    assert!(symlink.edges().is_empty());
    assert!(!discovered
        .artifacts()
        .iter()
        .any(|entry| entry.source().contains("secret")));
}
