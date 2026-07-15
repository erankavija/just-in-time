//! Read-only, byte-exact repository validation.
//!
//! A [`RepositoryView`] is the sole I/O boundary for this pipeline. The
//! filesystem implementation preserves ordinary validation behavior, while an
//! [`OverlayRepositoryView`] substitutes planned final bytes (including
//! deletions) without copying a repository or allowing a parser to reopen the
//! live storage root.

use crate::config::{JitConfig, ProjectionMode};
use crate::config_manager::ConfigManager;
use crate::document::content_parser_for;
use crate::domain::item::{
    expand_sugar_address, index_items, index_project_sources, is_qualified_reference,
    load_toml_scope_items, parse_kind_segmented_address, resolve_item_kinds, AddressScope,
    ProjectSource, RawScopeItem,
};
use crate::domain::{Event, EventTag, Issue};
use crate::graph::DependencyGraph;
use crate::storage::GateRegistry;
use crate::validation::invariants::InvariantRegistry;
use crate::validation::projection::{render_invariants_markdown, splice_region};
use crate::validation::rules::{RuleConfigError, RuleSet, SchemaSource, Severity};
use crate::validation::rules_gates_projection::render_rules_and_gates_markdown;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Error as IoError, ErrorKind};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const SUPPORTED_INDEX_SCHEMA_VERSION: u32 = 2;

/// One named stage in whole-repository validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepositoryValidationPass {
    /// `config.toml`, `templates.toml`, and `invariants.toml` parse and agree.
    EffectiveConfig,
    /// `rules.toml` and every referenced `schemas/*.json` load from the view.
    RulesAndSchemas,
    /// `gates.toml` parses and gate identities are unique.
    Gates,
    /// `index.json`, issue JSON, and `events.jsonl` are internally consistent.
    Records,
    /// Dependencies, required gates, DAG shape, and transitive reduction hold.
    RepositoryIntegrity,
    /// Effective local/graph rules enforce namespace and hierarchy policy.
    NamespaceAndHierarchy,
    /// Generic qualified item links resolve against view-derived item indexes.
    ItemLinks,
    /// Configured invariant and rules/gates projections match their view inputs.
    Projections,
}

/// Successful validation, including the stages that consumed the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryValidationReport {
    /// Passes completed in deterministic pipeline order.
    pub passes: Vec<RepositoryValidationPass>,
    /// Number of issue records validated.
    pub issue_count: usize,
    /// Number of known event records validated.
    pub event_count: usize,
}

/// A read-only repository byte source.
///
/// Paths are relative to the repository root. Implementations must never follow
/// an absolute or parent-traversing input. `list_files` returns regular-file
/// paths in deterministic order.
pub trait RepositoryView: Send + Sync {
    /// Repository root used only for diagnostics and synthetic schema paths.
    fn repository_root(&self) -> &Path;

    /// Read one repository-relative file, or `None` when it is absent.
    fn read_file(&self, relative: &Path) -> Result<Option<Vec<u8>>>;

    /// List regular files recursively beneath a repository-relative directory.
    fn list_files(&self, relative_dir: &Path) -> Result<Vec<PathBuf>>;
}

/// A repository view backed directly by the working tree filesystem.
#[derive(Debug, Clone)]
pub struct FilesystemRepositoryView {
    root: PathBuf,
    jit_root: PathBuf,
}

impl FilesystemRepositoryView {
    /// Create a filesystem view rooted at the repository containing `.jit`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let jit_root = root.join(".jit");
        Self { root, jit_root }
    }

    /// Create a filesystem view for an explicitly selected JIT data directory.
    /// The view still exposes its contents under the canonical virtual `.jit/`
    /// prefix, so custom `JIT_DATA_DIR` repositories execute the same pipeline.
    pub fn from_jit_root(jit_root: impl Into<PathBuf>) -> Result<Self> {
        let jit_root = jit_root.into();
        let root = jit_root
            .parent()
            .ok_or_else(|| {
                anyhow!(
                    "JIT data directory '{}' has no repository parent",
                    jit_root.display()
                )
            })?
            .to_path_buf();
        Ok(Self { root, jit_root })
    }

    fn resolve(&self, relative: &Path) -> PathBuf {
        relative.strip_prefix(".jit").map_or_else(
            |_| self.root.join(relative),
            |inside| self.jit_root.join(inside),
        )
    }
}

impl RepositoryView for FilesystemRepositoryView {
    fn repository_root(&self) -> &Path {
        &self.root
    }

    fn read_file(&self, relative: &Path) -> Result<Option<Vec<u8>>> {
        validate_relative(relative)?;
        let path = self.resolve(relative);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    fn list_files(&self, relative_dir: &Path) -> Result<Vec<PathBuf>> {
        validate_relative(relative_dir)?;
        let start = self.resolve(relative_dir);
        if !start.exists() {
            return Ok(Vec::new());
        }
        let mut pending = vec![start];
        let mut files = Vec::new();
        while let Some(dir) = pending.pop() {
            let entries =
                std::fs::read_dir(&dir).with_context(|| format!("listing {}", dir.display()))?;
            for entry in entries {
                let entry = entry?;
                let file_type = entry.file_type()?;
                if file_type.is_dir() {
                    pending.push(entry.path());
                } else if file_type.is_file() {
                    let path = entry.path();
                    let relative = if path.starts_with(&self.jit_root) {
                        PathBuf::from(".jit").join(path.strip_prefix(&self.jit_root)?)
                    } else {
                        path.strip_prefix(&self.root)?.to_path_buf()
                    };
                    files.push(relative);
                }
            }
        }
        files.sort();
        Ok(files)
    }
}

/// A layered final-state view over another repository view.
///
/// Map values are `Some(bytes)` for replacement/creation and `None` for a
/// planned deletion. Every read and directory listing consults the overlay
/// first, so all validators see one coherent proposed repository.
#[derive(Clone)]
pub struct OverlayRepositoryView {
    base: Arc<dyn RepositoryView>,
    changes: BTreeMap<PathBuf, Option<Vec<u8>>>,
}

impl OverlayRepositoryView {
    /// Layer repository-relative planned bytes over `base`.
    pub fn new(
        base: Arc<dyn RepositoryView>,
        changes: impl IntoIterator<Item = (PathBuf, Option<Vec<u8>>)>,
    ) -> Result<Self> {
        let changes = changes
            .into_iter()
            .map(|(path, bytes)| {
                validate_relative(&path)?;
                Ok((path, bytes))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(Self { base, changes })
    }
}

impl RepositoryView for OverlayRepositoryView {
    fn repository_root(&self) -> &Path {
        self.base.repository_root()
    }

    fn read_file(&self, relative: &Path) -> Result<Option<Vec<u8>>> {
        validate_relative(relative)?;
        self.changes
            .get(relative)
            .cloned()
            .map_or_else(|| self.base.read_file(relative), Ok)
    }

    fn list_files(&self, relative_dir: &Path) -> Result<Vec<PathBuf>> {
        validate_relative(relative_dir)?;
        let mut files: BTreeSet<PathBuf> =
            self.base.list_files(relative_dir)?.into_iter().collect();
        for (path, bytes) in &self.changes {
            if path.starts_with(relative_dir) {
                if bytes.is_some() {
                    files.insert(path.clone());
                } else {
                    files.remove(path);
                }
            }
        }
        Ok(files.into_iter().collect())
    }
}

/// Validate the exact repository exposed by `view` through the full read-only
/// pipeline.
pub fn validate_repository(view: &dyn RepositoryView) -> Result<RepositoryValidationReport> {
    let mut passes = Vec::new();
    let config = load_config(view).context("effective-config validation pass")?;
    passes.push(RepositoryValidationPass::EffectiveConfig);

    let namespaces =
        ConfigManager::new(view.repository_root().join(".jit")).namespaces_from_config(&config);
    let rules =
        load_rules(view, &config, &namespaces).context("rules-and-schemas validation pass")?;
    passes.push(RepositoryValidationPass::RulesAndSchemas);

    let gates = load_gates(view).context("gates validation pass")?;
    passes.push(RepositoryValidationPass::Gates);

    let records = load_records(view).context("records validation pass")?;
    passes.push(RepositoryValidationPass::Records);

    validate_integrity(view, &records.issues, &gates)
        .context("repository-integrity validation pass")?;
    passes.push(RepositoryValidationPass::RepositoryIntegrity);

    validate_rules(view, &records.issues, &rules, &namespaces, &config)
        .context("namespace-and-hierarchy validation pass")?;
    validate_enforcement_drift(&config, &rules, &gates)
        .context("namespace-and-hierarchy validation pass")?;
    passes.push(RepositoryValidationPass::NamespaceAndHierarchy);

    validate_item_links(view, &records.issues, &config).context("item-links validation pass")?;
    passes.push(RepositoryValidationPass::ItemLinks);

    validate_projections(view, &config, &rules, &gates).context("projections validation pass")?;
    passes.push(RepositoryValidationPass::Projections);

    Ok(RepositoryValidationReport {
        passes,
        issue_count: records.issues.len(),
        event_count: records.event_count,
    })
}

fn validate_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(anyhow!(
            "repository view path '{}' must be a non-empty relative normal path",
            path.display()
        ));
    }
    Ok(())
}

fn read_text(view: &dyn RepositoryView, path: &str) -> Result<Option<String>> {
    view.read_file(Path::new(path))?
        .map(|bytes| String::from_utf8(bytes).context(format!("{path} is not UTF-8")))
        .transpose()
}

fn required_text(view: &dyn RepositoryView, path: &str) -> Result<String> {
    read_text(view, path)?.ok_or_else(|| anyhow!("required repository file '{path}' is missing"))
}

fn load_config(view: &dyn RepositoryView) -> Result<JitConfig> {
    let mut config: JitConfig = toml::from_str(
        read_text(view, ".jit/config.toml")?
            .as_deref()
            .unwrap_or(""),
    )
    .context("invalid .jit/config.toml")?;
    let hierarchy_types: Vec<&str> = config
        .type_hierarchy
        .as_ref()
        .map(|hierarchy| hierarchy.types.keys().map(String::as_str).collect())
        .unwrap_or_default();
    config.templates = match read_text(view, ".jit/templates.toml")? {
        Some(content) => {
            crate::templates::TemplateRegistry::from_toml_str(&content, &hierarchy_types)
                .context("invalid .jit/templates.toml")?
        }
        None => crate::templates::TemplateRegistry::empty(),
    };
    config.invariants = match read_text(view, ".jit/invariants.toml")? {
        Some(content) => {
            InvariantRegistry::from_toml_str(&content).context("invalid .jit/invariants.toml")?
        }
        None => InvariantRegistry::empty(),
    };
    config
        .validate_item_kinds()
        .context("invalid [item_kinds] in .jit/config.toml")?;
    Ok(config)
}

fn load_rules(
    view: &dyn RepositoryView,
    config: &JitConfig,
    namespaces: &crate::domain::LabelNamespaces,
) -> Result<RuleSet> {
    let Some(content) = read_text(view, ".jit/rules.toml")? else {
        return Ok(crate::validation::defaults::default_ruleset(namespaces));
    };
    let jit_root = view.repository_root().join(".jit");
    let parsed = RuleSet::from_toml_str_with_loaders(
        &content,
        &jit_root,
        Some(config),
        |rule, reference| {
            let path = format!(".jit/{reference}");
            let synthetic_path = jit_root.join(&reference);
            let bytes =
                view.read_file(Path::new(&path))
                    .map_err(|error| RuleConfigError::SchemaIo {
                        rule: rule.to_string(),
                        path: synthetic_path.clone(),
                        source: IoError::other(error.to_string()),
                    })?;
            let content = bytes.ok_or_else(|| RuleConfigError::SchemaIo {
                rule: rule.to_string(),
                path: synthetic_path.clone(),
                source: IoError::new(ErrorKind::NotFound, "schema absent from repository view"),
            })?;
            let schema =
                serde_json::from_slice(&content).map_err(|source| RuleConfigError::SchemaJson {
                    rule: rule.to_string(),
                    path: synthetic_path.clone(),
                    source,
                })?;
            Ok(SchemaSource {
                reference,
                path: synthetic_path,
                schema,
            })
        },
    )?;
    Ok(crate::validation::defaults::reconcile_default_rules_with_config(parsed, namespaces))
}

#[derive(Deserialize)]
struct GatesFile {
    #[serde(default)]
    gates: Vec<crate::domain::Gate>,
}

fn load_gates(view: &dyn RepositoryView) -> Result<GateRegistry> {
    let content = read_text(view, ".jit/gates.toml")?.unwrap_or_default();
    let file: GatesFile = toml::from_str(&content).context("invalid .jit/gates.toml")?;
    let mut gates = HashMap::new();
    for gate in file.gates {
        let key = gate.key.clone();
        if gates.insert(key.clone(), gate).is_some() {
            return Err(anyhow!("duplicate gate key '{key}' in .jit/gates.toml"));
        }
    }
    Ok(GateRegistry { gates })
}

#[derive(Deserialize)]
struct RepositoryIndex {
    schema_version: u32,
    #[serde(default)]
    all_ids: Vec<String>,
    #[serde(default)]
    deleted_ids: Vec<String>,
}

struct Records {
    issues: Vec<Issue>,
    event_count: usize,
}

fn load_records(view: &dyn RepositoryView) -> Result<Records> {
    let index: RepositoryIndex = serde_json::from_str(&required_text(view, ".jit/index.json")?)
        .context("invalid .jit/index.json")?;
    if index.schema_version > SUPPORTED_INDEX_SCHEMA_VERSION {
        return Err(anyhow!(
            "repository index schema {} is newer than supported schema {}",
            index.schema_version,
            SUPPORTED_INDEX_SCHEMA_VERSION
        ));
    }
    let mut seen = HashSet::new();
    if let Some(id) = index.all_ids.iter().find(|id| !seen.insert(id.as_str())) {
        return Err(anyhow!("duplicate issue id '{id}' in .jit/index.json"));
    }
    if let Some(id) = index
        .deleted_ids
        .iter()
        .find(|id| seen.contains(id.as_str()))
    {
        return Err(anyhow!(
            "issue id '{id}' is both live and deleted in .jit/index.json"
        ));
    }
    let issues = index
        .all_ids
        .iter()
        .map(|id| {
            let path = format!(".jit/issues/{id}.json");
            let issue: Issue = serde_json::from_str(&required_text(view, &path)?)
                .with_context(|| format!("invalid {path}"))?;
            if issue.id != *id {
                return Err(anyhow!(
                    "issue file '{path}' contains id '{}' instead of '{id}'",
                    issue.id
                ));
            }
            Ok(issue)
        })
        .collect::<Result<Vec<_>>>()?;
    let issue_files = view.list_files(Path::new(".jit/issues"))?;
    let expected: BTreeSet<PathBuf> = index
        .all_ids
        .iter()
        .map(|id| PathBuf::from(format!(".jit/issues/{id}.json")))
        .collect();
    let actual: BTreeSet<PathBuf> = issue_files
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    if actual != expected {
        return Err(anyhow!(
            "issue files disagree with .jit/index.json (expected {expected:?}, found {actual:?})"
        ));
    }

    let mut event_count = 0;
    if let Some(events) = read_text(view, ".jit/events.jsonl")? {
        for (offset, line) in events.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(line)
                .with_context(|| format!("invalid .jit/events.jsonl line {}", offset + 1))?;
            let tag = value
                .get("type")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow!("event line {} is missing a string type", offset + 1))?;
            if EventTag::ALL.iter().any(|known| known.as_str() == tag) {
                let _: Event = serde_json::from_value(value).with_context(|| {
                    format!(
                        "invalid known event on .jit/events.jsonl line {}",
                        offset + 1
                    )
                })?;
                event_count += 1;
            }
        }
    }
    Ok(Records {
        issues,
        event_count,
    })
}

fn validate_integrity(
    view: &dyn RepositoryView,
    issues: &[Issue],
    gates: &GateRegistry,
) -> Result<()> {
    let ids: HashSet<&str> = issues.iter().map(|issue| issue.id.as_str()).collect();
    for issue in issues {
        for dependency in &issue.dependencies {
            if !ids.contains(dependency.as_str()) {
                return Err(anyhow!(
                    "issue '{}' depends on missing issue '{dependency}'",
                    issue.id
                ));
            }
        }
        for gate in &issue.gates_required {
            if !gates.gates.contains_key(gate) {
                return Err(anyhow!(
                    "issue '{}' requires undefined gate '{gate}'",
                    issue.id
                ));
            }
        }
        for document in &issue.documents {
            if let Some(reference) = document.commit.as_deref() {
                let repository = git2::Repository::open(view.repository_root())?;
                let object = repository.revparse_single(reference)?;
                object
                    .peel_to_commit()?
                    .tree()?
                    .get_path(Path::new(&document.path))?;
            } else if view.read_file(Path::new(&document.path))?.is_none() {
                let in_head = git2::Repository::open(view.repository_root())
                    .and_then(|repository| {
                        let object = repository.revparse_single("HEAD")?;
                        let commit = object.peel_to_commit()?;
                        let tree = commit.tree()?;
                        tree.get_path(Path::new(&document.path)).map(|_| ())
                    })
                    .is_ok();
                if !in_head {
                    return Err(anyhow!(
                        "issue '{}' references missing document '{}'",
                        issue.id,
                        document.path
                    ));
                }
            }
        }
    }
    let refs: Vec<&Issue> = issues.iter().collect();
    let graph = DependencyGraph::new(&refs);
    graph.validate_dag()?;
    if issues.len() > 1 && !graph.get_isolated_nodes().is_empty() {
        return Err(anyhow!("repository contains isolated issues"));
    }
    for issue in issues {
        let reduced = graph.compute_transitive_reduction(&issue.id);
        if issue
            .dependencies
            .iter()
            .any(|dependency| !reduced.contains(dependency))
        {
            return Err(anyhow!(
                "issue '{}' contains a transitively redundant dependency",
                issue.id
            ));
        }
    }
    Ok(())
}

fn validate_rules(
    view: &dyn RepositoryView,
    issues: &[Issue],
    rules: &RuleSet,
    namespaces: &crate::domain::LabelNamespaces,
    config: &JitConfig,
) -> Result<()> {
    let repo_format = config
        .validation
        .as_ref()
        .map_or(Ok(crate::domain::ContentFormat::Markdown), |validation| {
            validation.content_format()
        })?;
    for issue in issues {
        let evaluation = crate::validation::evaluate_local(issue, rules, repo_format)?;
        if let Some(finding) = evaluation
            .findings()
            .into_iter()
            .find(|finding| finding.severity == Severity::Error)
        {
            return Err(anyhow!(
                "issue {} fails rule '{}': {}",
                issue.short_id(),
                finding.rule,
                finding.message
            ));
        }
    }
    let graph_rules: Vec<_> = rules
        .rules
        .iter()
        .filter(|rule| rule.scope == crate::validation::rules::RuleScope::Graph)
        .collect();
    let hierarchy = crate::validation::defaults::hierarchy_config(namespaces);
    let plan_content = resolve_plan_content(view, issues, config)?;
    let findings = crate::validation::graph::evaluate_graph(
        &graph_rules,
        issues,
        &hierarchy,
        repo_format,
        chrono::Utc::now(),
        &plan_content,
    );
    if let Some(finding) = findings
        .iter()
        .find(|finding| finding.finding.severity == Severity::Error)
    {
        return Err(anyhow!(
            "graph rule '{}' failed: {}",
            finding.finding.rule,
            finding.finding.message
        ));
    }
    Ok(())
}

fn validate_enforcement_drift(
    config: &JitConfig,
    rules: &RuleSet,
    gates: &GateRegistry,
) -> Result<()> {
    use crate::validation::drift::{enforcement_drift_tolerant, SourceState};

    let rule_names: BTreeSet<&str> = rules.rules.iter().map(|rule| rule.name.as_str()).collect();
    let gate_keys: BTreeSet<&str> = gates.gates.keys().map(String::as_str).collect();
    let findings = enforcement_drift_tolerant(
        &config.invariants.invariants,
        SourceState::Loaded(&rule_names),
        SourceState::Loaded(&gate_keys),
    );
    if let Some(finding) = findings.first() {
        return Err(anyhow!(finding.message()));
    }
    Ok(())
}

fn resolve_plan_content(
    view: &dyn RepositoryView,
    issues: &[Issue],
    config: &JitConfig,
) -> Result<HashMap<String, String>> {
    let templates = &config.templates;
    let breakable: HashSet<String> = templates.breakable_types().into_iter().collect();
    let by_id: HashMap<&str, &Issue> = issues
        .iter()
        .map(|issue| (issue.id.as_str(), issue))
        .collect();
    let mut content = HashMap::new();
    for issue in issues {
        let Some(issue_type) = crate::labels::type_label_value(&issue.labels)
            .filter(|issue_type| breakable.contains(*issue_type))
        else {
            continue;
        };
        let Some(template) = templates.template_for_container(issue_type) else {
            continue;
        };
        if template.plan_doc_location(&templates.roles).is_none() {
            continue;
        }
        let planning =
            crate::commands::find_planning_node(issue, template, &templates.roles, &by_id);
        let Some(path) = planning.and_then(crate::commands::planning_node_plan_path) else {
            continue;
        };
        match read_text(view, &path)? {
            Some(plan) => {
                content.insert(issue.id.clone(), plan);
            }
            None if planning.is_none_or(|node| node.state != crate::domain::State::Done) => {}
            None => return Err(anyhow!("required plan document '{path}' is missing")),
        }
    }
    Ok(content)
}

fn validate_item_links(
    view: &dyn RepositoryView,
    issues: &[Issue],
    config: &JitConfig,
) -> Result<()> {
    let kinds = resolve_item_kinds(config.item_kinds.as_ref())?;
    if kinds.is_empty() {
        return Ok(());
    }
    let repo_format = config
        .validation
        .as_ref()
        .map_or(Ok(crate::domain::ContentFormat::Markdown), |validation| {
            validation.content_format()
        })?;
    let mut addresses = HashSet::new();
    let issue_kinds: Vec<_> = kinds
        .iter()
        .filter(|kind| !kind.kind_scope().is_project())
        .cloned()
        .collect();
    for issue in issues {
        let parser = content_parser_for(issue.content_format, repo_format)?;
        addresses.extend(
            index_items(issue, &issue_kinds, parser.as_ref())?
                .into_iter()
                .map(|item| item.qualified_id),
        );
    }
    let mut markdown_sources = Vec::new();
    let mut registry_items: Vec<RawScopeItem> = Vec::new();
    for kind in kinds.iter().filter(|kind| kind.kind_scope().is_project()) {
        if let Some(descriptor) = kind.toml_source() {
            if let Some(content) = read_text(view, &descriptor.toml)? {
                registry_items.extend(load_toml_scope_items(kind.name(), descriptor, &content)?);
            }
        } else if let Some(path) = kind.source() {
            if let Some(markdown) = read_text(view, path)? {
                markdown_sources.push(ProjectSource {
                    kind: kind.clone(),
                    markdown,
                });
            }
        }
    }
    let parser = content_parser_for(None, repo_format)?;
    addresses.extend(
        index_project_sources(&markdown_sources, registry_items, parser.as_ref())?
            .into_iter()
            .map(|item| item.qualified_id),
    );
    let link_namespaces: HashSet<&str> = kinds
        .iter()
        .flat_map(|kind| kind.link_namespaces().iter().map(String::as_str))
        .collect();
    for issue in issues {
        for label in &issue.labels {
            let Some((namespace, value)) = label.split_once(':') else {
                continue;
            };
            if !link_namespaces.contains(namespace) || !is_qualified_reference(value) {
                continue;
            }
            let address = if value.starts_with('@') {
                parse_kind_segmented_address(value)?
            } else {
                expand_sugar_address(value, &kinds)?
            };
            let canonical = match address.scope {
                AddressScope::Project => {
                    format!("@/{}/{}", address.kind, address.self_id)
                }
                AddressScope::NamedProject(name) => {
                    let declared = config
                        .project
                        .as_ref()
                        .and_then(|project| project.name.as_ref())
                        .map(crate::config::ProjectName::as_str);
                    if declared != Some(name.as_str()) {
                        return Err(anyhow!(
                            "item link '{label}' addresses project '{name}', not the local project"
                        ));
                    }
                    format!("@/{}/{}", address.kind, address.self_id)
                }
                AddressScope::Issue(issue_ref) => {
                    let matches: Vec<&Issue> = issues
                        .iter()
                        .filter(|candidate| candidate.id.starts_with(&issue_ref))
                        .collect();
                    let [owner] = matches.as_slice() else {
                        return Err(anyhow!(
                            "item link '{label}' has an unresolved or ambiguous issue scope '{issue_ref}'"
                        ));
                    };
                    format!(
                        "@/issue/{}/{}/{}",
                        owner.short_id(),
                        address.kind,
                        address.self_id
                    )
                }
            };
            if !addresses.contains(&canonical) {
                return Err(anyhow!(
                    "issue {} has dangling item link '{label}'",
                    issue.short_id()
                ));
            }
        }
    }
    Ok(())
}

fn projected_content(
    view: &dyn RepositoryView,
    target: &str,
    mode: ProjectionMode,
    rendered: &str,
    begin: &str,
    end: &str,
) -> Result<String> {
    match mode {
        ProjectionMode::SeparateFile => Ok(rendered.to_string()),
        ProjectionMode::Region => {
            let current = read_text(view, target)?
                .ok_or_else(|| anyhow!("projection target '{target}' is missing"))?;
            Ok(splice_region(&current, rendered, begin, end)?)
        }
    }
}

fn validate_projections(
    view: &dyn RepositoryView,
    config: &JitConfig,
    rules: &RuleSet,
    gates: &GateRegistry,
) -> Result<()> {
    if let Some(projection) = config.invariant_projection.as_ref() {
        let rendered = render_invariants_markdown(&config.invariants, projection.style());
        let expected = projected_content(
            view,
            projection.target(),
            projection.mode(),
            &rendered,
            projection.region_begin(),
            projection.region_end(),
        )?;
        let actual = read_text(view, projection.target())?
            .ok_or_else(|| anyhow!("projection target '{}' is missing", projection.target()))?;
        if actual != expected {
            return Err(anyhow!(
                "invariant projection '{}' is stale",
                projection.target()
            ));
        }
    }
    if let Some(projection) = config.rules_gates_projection.as_ref() {
        let rendered = render_rules_and_gates_markdown(rules, gates, projection.style());
        let expected = projected_content(
            view,
            projection.target(),
            projection.mode(),
            &rendered,
            projection.region_begin(),
            projection.region_end(),
        )?;
        let actual = read_text(view, projection.target())?
            .ok_or_else(|| anyhow!("projection target '{}' is missing", projection.target()))?;
        if actual != expected {
            return Err(anyhow!(
                "rules/gates projection '{}' is stale",
                projection.target()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{IssueStore, JsonFileStorage};

    fn fixture() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        let store = JsonFileStorage::new(repo.path().join(".jit"));
        store.init().unwrap();
        let mut issue = Issue::new(
            "Root".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n".to_string(),
        );
        issue.labels = vec!["type:task".to_string()];
        store.save_issue(issue).unwrap();
        std::fs::write(
            repo.path().join(".jit/config.toml"),
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        repo
    }

    fn overlay(
        repo: &tempfile::TempDir,
        changes: impl IntoIterator<Item = (&'static str, Option<&'static str>)>,
    ) -> OverlayRepositoryView {
        OverlayRepositoryView::new(
            Arc::new(FilesystemRepositoryView::new(repo.path())),
            changes.into_iter().map(|(path, content)| {
                (
                    PathBuf::from(path),
                    content.map(|text| text.as_bytes().to_vec()),
                )
            }),
        )
        .unwrap()
    }

    #[test]
    fn test_filesystem_and_identity_overlay_preserve_validation_results() {
        let repo = fixture();
        let filesystem = FilesystemRepositoryView::new(repo.path());
        let plain = validate_repository(&filesystem).unwrap();
        let planned = validate_repository(&overlay(&repo, [])).unwrap();
        assert_eq!(plain, planned);
        assert_eq!(plain.passes.len(), 8);
    }

    #[test]
    fn test_live_repository_filesystem_view_regression() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        validate_repository(&FilesystemRepositoryView::new(root)).unwrap();
    }

    #[test]
    fn test_overlay_config_templates_and_rules_ignore_disagreeing_live_bytes() {
        let repo = fixture();
        std::fs::write(repo.path().join(".jit/config.toml"), "not toml = [").unwrap();
        let view = overlay(
            &repo,
            [
                (
                    ".jit/config.toml",
                    Some("[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n"),
                ),
                (".jit/templates.toml", Some("templates = []\n")),
                (".jit/rules.toml", Some("")),
            ],
        );
        assert!(validate_repository(&view).is_ok());
    }

    #[test]
    fn test_overlay_rules_and_referenced_schema_ignore_disagreeing_live_schema() {
        let repo = fixture();
        let rules = "[[rules]]\nname = \"planned-schema\"\nseverity = \"error\"\nassert = { json-schema = \"schemas/planned.json\" }\n";
        std::fs::create_dir_all(repo.path().join(".jit/schemas")).unwrap();
        std::fs::write(repo.path().join(".jit/rules.toml"), rules).unwrap();
        std::fs::write(repo.path().join(".jit/schemas/planned.json"), "not json").unwrap();
        let view = overlay(
            &repo,
            [
                (".jit/rules.toml", Some(rules)),
                (".jit/schemas/planned.json", Some("{}")),
            ],
        );
        assert!(validate_repository(&view).is_ok());
        let live_error = validate_repository(&FilesystemRepositoryView::new(repo.path()))
            .unwrap_err()
            .to_string();
        assert!(live_error.contains("rules-and-schemas"), "{live_error}");
    }

    #[test]
    fn test_overlay_gate_registry_ignores_disagreeing_live_registry() {
        let repo = fixture();
        std::fs::write(repo.path().join(".jit/gates.toml"), "not toml = [").unwrap();
        let view = overlay(&repo, [(".jit/gates.toml", Some(""))]);
        assert!(validate_repository(&view).is_ok());
    }

    #[test]
    fn test_overlay_index_events_and_issue_records_ignore_disagreeing_live_bytes() {
        let repo = fixture();
        let filesystem = FilesystemRepositoryView::new(repo.path());
        let index = required_text(&filesystem, ".jit/index.json").unwrap();
        let events = read_text(&filesystem, ".jit/events.jsonl")
            .unwrap()
            .unwrap_or_default();
        std::fs::write(repo.path().join(".jit/index.json"), "not json").unwrap();
        std::fs::write(repo.path().join(".jit/events.jsonl"), "not jsonl").unwrap();
        let view = OverlayRepositoryView::new(
            Arc::new(FilesystemRepositoryView::new(repo.path())),
            [
                (PathBuf::from(".jit/index.json"), Some(index.into_bytes())),
                (
                    PathBuf::from(".jit/events.jsonl"),
                    Some(events.into_bytes()),
                ),
            ],
        )
        .unwrap();
        assert!(validate_repository(&view).is_ok());
    }

    #[test]
    fn test_overlay_records_dag_and_gate_integrity_judge_planned_bytes() {
        let repo = fixture();
        let index = required_text(
            &FilesystemRepositoryView::new(repo.path()),
            ".jit/index.json",
        )
        .unwrap();
        let id = serde_json::from_str::<RepositoryIndex>(&index)
            .unwrap()
            .all_ids
            .remove(0);
        let mut issue: Issue = serde_json::from_str(
            &required_text(
                &FilesystemRepositoryView::new(repo.path()),
                &format!(".jit/issues/{id}.json"),
            )
            .unwrap(),
        )
        .unwrap();
        issue.dependencies = vec!["missing".to_string()];
        issue.gates_required = vec!["planned".to_string()];
        let bad = serde_json::to_vec_pretty(&issue).unwrap();
        let view = OverlayRepositoryView::new(
            Arc::new(FilesystemRepositoryView::new(repo.path())),
            [
                (PathBuf::from(format!(".jit/issues/{id}.json")), Some(bad)),
                (
                    PathBuf::from(".jit/gates.toml"),
                    Some(b"[[gates]]\nkey = \"planned\"\ntitle = \"p\"\ndescription = \"\"\nstage = \"postcheck\"\nmode = \"manual\"\n".to_vec()),
                ),
            ],
        )
        .unwrap();
        let error = format!("{:#}", validate_repository(&view).unwrap_err());
        assert!(
            error.contains("repository-integrity") && error.contains("missing"),
            "{error}"
        );
    }

    #[test]
    fn test_overlay_namespace_and_hierarchy_judge_planned_config() {
        let repo = fixture();
        let view = overlay(
            &repo,
            [(
                ".jit/config.toml",
                Some("[type_hierarchy.types]\nepic = 2\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n"),
            )],
        );
        let error = format!("{:#}", validate_repository(&view).unwrap_err());
        assert!(error.contains("namespace-and-hierarchy"), "{error}");
    }

    #[test]
    fn test_overlay_item_links_and_projection_use_planned_sources() {
        let repo = fixture();
        let config = "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n[namespaces.enforces]\ndescription = \"Item link\"\nunique = false\n\
            [item_kinds.invariant]\nsection = \"success_criteria\"\nid-pattern = \"[a-z-]+\"\nmarkers = []\nlink-namespaces = [\"enforces\"]\nscope = \"project\"\nsource-of-truth = \"registry-first\"\nsource = { toml = \".jit/invariants.toml\", table = \"invariants\", id-field = \"id\", text-field = \"statement\" }\n\
            [invariant_projection]\ntarget = \"INVARIANTS.md\"\nmode = \"separate-file\"\n";
        let invariants = "[[invariants]]\nid = \"planned\"\nstatement = \"planned bytes win\"\nkind = \"advisory\"\n";
        let registry = InvariantRegistry::from_toml_str(invariants).unwrap();
        let rendered = render_invariants_markdown(
            &registry,
            crate::config::InvariantProjectionConfig::default().style(),
        );
        let filesystem = FilesystemRepositoryView::new(repo.path());
        let index: RepositoryIndex =
            serde_json::from_str(&required_text(&filesystem, ".jit/index.json").unwrap()).unwrap();
        let id = &index.all_ids[0];
        let mut issue: Issue = serde_json::from_str(
            &required_text(&filesystem, &format!(".jit/issues/{id}.json")).unwrap(),
        )
        .unwrap();
        issue
            .labels
            .push("enforces:@/invariant/planned".to_string());
        let view = OverlayRepositoryView::new(
            Arc::new(FilesystemRepositoryView::new(repo.path())),
            [
                (
                    PathBuf::from(".jit/config.toml"),
                    Some(config.as_bytes().to_vec()),
                ),
                (
                    PathBuf::from(".jit/invariants.toml"),
                    Some(invariants.as_bytes().to_vec()),
                ),
                (PathBuf::from("INVARIANTS.md"), Some(rendered.into_bytes())),
                (
                    PathBuf::from(format!(".jit/issues/{id}.json")),
                    Some(serde_json::to_vec_pretty(&issue).unwrap()),
                ),
            ],
        )
        .unwrap();
        validate_repository(&view).unwrap();
    }

    #[test]
    fn test_overlay_projection_rejects_stale_planned_target_despite_live_state() {
        let repo = fixture();
        let config = "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n[invariant_projection]\ntarget = \"INVARIANTS.md\"\nmode = \"separate-file\"\n";
        let invariants = "[[invariants]]\nid = \"planned\"\nstatement = \"planned bytes win\"\nkind = \"advisory\"\n";
        let view = overlay(
            &repo,
            [
                (".jit/config.toml", Some(config)),
                (".jit/invariants.toml", Some(invariants)),
                ("INVARIANTS.md", Some("stale planned projection\n")),
            ],
        );
        let error = validate_repository(&view).unwrap_err().to_string();
        assert!(error.contains("projections validation pass"), "{error}");
    }
}
