//! Read-only, byte-exact repository validation.
//!
//! Whole-repository validation ([`validate_repository`]) reads exclusively from a
//! closed [`RepositoryImage`](crate::repository_state::RepositoryImage): a live
//! capture validates the working tree and an overlay image validates a proposed
//! final state, with no live filesystem or Git I/O in any pass. Every pass reads
//! image-projected bytes through one byte-read closure, so no path forks its
//! config/rules/gates loaders.

use crate::config::{JitConfig, ProjectionMode};
use crate::declarations::invariants::InvariantRegistry;
use crate::declarations::rules::{RuleConfigError, RuleSet, Severity};
use crate::declarations::GateChecker;
use crate::declarations::GateRegistry;
use crate::document::content_parser_for;
use crate::domain::item::{
    expand_sugar_address, index_items, index_project_sources, is_qualified_reference,
    load_toml_scope_items, parse_kind_segmented_address, resolve_item_kinds, AddressScope,
    ProjectSource, RawScopeItem,
};
use crate::domain::{parse_known_events, Issue, SHORT_ID_LENGTH};
use crate::graph::DependencyGraph;
use crate::repository_state::{
    render_projection_body, require_target, splice_region, ProjectionInputs, RepositoryEntry,
    RepositoryImage,
};
use crate::validation::engine::Finding;
use crate::validation::report::{ReportedFinding, RuleReport};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};

const SUPPORTED_INDEX_SCHEMA_VERSION: u32 = 2;

/// One named stage in whole-repository validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepositoryValidationPass {
    /// `config.toml`, `templates.toml`, and `invariants.toml` parse and agree.
    EffectiveConfig,
    /// `rules.toml` and referenced `schemas/*.json` are inspected from the view.
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

/// Validation outcome, including semantic findings and stages reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryValidationReport {
    /// Stages reached in deterministic pipeline order.
    pub passes: Vec<RepositoryValidationPass>,
    /// Number of issue records validated.
    pub issue_count: usize,
    /// Number of known event records validated.
    pub event_count: usize,
    /// Declarative and built-in semantic findings produced from the same view.
    pub rule_report: RuleReport,
}

/// A structural repository-validation failure with all semantic findings that
/// could still be collected from the already captured [`RepositoryImage`].
///
/// Callers that render validation output should inspect [`Self::report`] before
/// propagating [`Self::into_error`]. This keeps a structural failure authoritative
/// without discarding rule findings or rerunning rules through another data source.
#[derive(Debug)]
pub struct RepositoryValidationFailure {
    error: anyhow::Error,
    report: RepositoryValidationReport,
}

impl RepositoryValidationFailure {
    fn new(error: anyhow::Error, report: RepositoryValidationReport) -> Self {
        Self { error, report }
    }

    /// Partial validation report collected from the exact supplied view.
    pub fn report(&self) -> &RepositoryValidationReport {
        &self.report
    }

    /// Consume the failure into its structural error and partial report.
    pub fn into_parts(self) -> (anyhow::Error, RepositoryValidationReport) {
        (self.error, self.report)
    }

    /// Consume the failure into its authoritative structural error.
    pub fn into_error(self) -> anyhow::Error {
        self.error
    }
}

impl std::fmt::Display for RepositoryValidationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:#}", self.error)
    }
}

impl std::error::Error for RepositoryValidationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

/// Read a repo-relative path's bytes from the captured image.
///
/// A `.jit/`-prefixed path is a `Data(...)` entry, every other repo-relative path
/// a `Worktree(...)` entry. `Ok(None)` is a captured absence; a path outside the
/// captured closure fails typed as `UndiscoveredRepositoryPath` (closed-read
/// discipline), never a silent absence.
fn image_read(image: &RepositoryImage, repo_rel: &str) -> Result<Option<Vec<u8>>> {
    use crate::repository_state::VirtualPath;
    let vpath = match repo_rel.strip_prefix(".jit/") {
        Some(rest) => VirtualPath::data(rest),
        None => VirtualPath::worktree(repo_rel),
    }?;
    Ok(image.file_bytes(&vpath)?.map(<[u8]>::to_vec))
}

/// Validate the exact repository captured in `image` through the full read-only
/// pipeline.
///
/// The image is the single closed evidence source (plan §2 two-phase capture): a
/// live capture validates the working tree, and an overlay image
/// ([`apply_overlay`](crate::repository_state::apply_overlay)) validates a proposed
/// final state. Every pass reads only image-projected content — no live filesystem
/// or Git I/O — so a path outside the captured closure fails typed rather than
/// reading through.
pub fn validate_repository(
    image: &RepositoryImage,
) -> std::result::Result<RepositoryValidationReport, RepositoryValidationFailure> {
    let read = move |path: &str| image_read(image, path);
    let read: &ReadBytes<'_> = &read;
    let mut passes = Vec::new();
    let config = match load_config(read).context("effective-config validation pass") {
        Ok(config) => config,
        Err(error) => {
            return Err(RepositoryValidationFailure::new(
                error,
                RepositoryValidationReport {
                    passes,
                    issue_count: 0,
                    event_count: 0,
                    rule_report: RuleReport::default(),
                },
            ));
        }
    };
    passes.push(RepositoryValidationPass::EffectiveConfig);

    let namespaces = crate::config_manager::namespaces_from_config(&config);
    let (rules, rules_loaded, mut findings) = match load_rules(read, &config, &namespaces) {
        Ok(rules) => (rules, true, Vec::new()),
        Err(error) => (
            RuleSet::empty(),
            false,
            vec![ReportedFinding::new(
                None,
                &Finding {
                    rule: "rules-file".to_string(),
                    severity: Severity::Error,
                    message: format!("config error: {error:#}"),
                },
            )],
        ),
    };
    passes.push(RepositoryValidationPass::RulesAndSchemas);

    let mut structural_error = None;
    let gates = match load_gates(read).context("gates validation pass") {
        Ok(gates) => {
            passes.push(RepositoryValidationPass::Gates);
            Some(gates)
        }
        Err(error) => {
            structural_error = Some(error);
            None
        }
    };

    let records = match load_records(read, image).context("records validation pass") {
        Ok(records) => {
            passes.push(RepositoryValidationPass::Records);
            Some(records)
        }
        Err(error) => {
            if structural_error.is_none() {
                structural_error = Some(error);
            }
            None
        }
    };

    if let (Some(records), Some(gates)) = (&records, &gates) {
        let integrity_error = validate_integrity(image, &records.issues, gates)
            .context("repository-integrity validation pass")
            .err();
        if let Some(error) = integrity_error {
            if structural_error.is_none() {
                structural_error = Some(error);
            }
        } else if structural_error.is_none() {
            structural_error = validate_machine_local_claims(image.layout().worktree_root())
                .context("repository-integrity validation pass")
                .err();
            if structural_error.is_none() {
                passes.push(RepositoryValidationPass::RepositoryIntegrity);
            }
        }
    }

    let mut namespace_and_hierarchy_complete = true;
    if rules_loaded {
        if let Some(records) = &records {
            match collect_rule_findings(read, &records.issues, &rules, &namespaces, &config)
                .context("namespace-and-hierarchy validation pass")
            {
                Ok(rule_findings) => findings.extend(rule_findings),
                Err(error) => {
                    namespace_and_hierarchy_complete = false;
                    if structural_error.is_none() {
                        structural_error = Some(error);
                    }
                }
            }
        } else {
            namespace_and_hierarchy_complete = false;
        }
    }
    if let Some(gates) = &gates {
        findings.extend(collect_enforcement_drift_findings(
            &config,
            rules_loaded.then_some(&rules),
            gates,
        ));
        findings.extend(collect_review_placeholder_findings(gates));
    } else {
        namespace_and_hierarchy_complete = false;
    }
    if namespace_and_hierarchy_complete {
        passes.push(RepositoryValidationPass::NamespaceAndHierarchy);
    }

    if let Some(records) = &records {
        match collect_item_link_findings(read, &records.issues, &config)
            .context("item-links validation pass")
        {
            Ok(item_findings) => {
                findings.extend(item_findings);
                passes.push(RepositoryValidationPass::ItemLinks);
            }
            Err(error) => {
                if structural_error.is_none() {
                    structural_error = Some(error);
                }
            }
        }
    }

    if rules_loaded {
        if let Some(gates) = &gates {
            match validate_projections(read, &config, &rules, gates)
                .context("projections validation pass")
            {
                Ok(()) => passes.push(RepositoryValidationPass::Projections),
                Err(error) => {
                    if structural_error.is_none() {
                        structural_error = Some(error);
                    }
                }
            }
        }
    } else if gates.is_some() {
        passes.push(RepositoryValidationPass::Projections);
    }

    let (issue_count, event_count) = records.as_ref().map_or((0, 0), |records| {
        (records.issues.len(), records.event_count)
    });
    let report = RepositoryValidationReport {
        passes,
        issue_count,
        event_count,
        rule_report: RuleReport { findings },
    };
    match structural_error {
        Some(error) => Err(RepositoryValidationFailure::new(error, report)),
        None => Ok(report),
    }
}

/// A repository byte source keyed by repo-relative path.
///
/// The single read abstraction the config/rules/gates loaders consume, so the
/// image-backed validation pipeline and the surviving view-backed projection
/// helpers share one loader implementation rather than forking. `Ok(None)` is a
/// captured/present absence; an image source returns `Err` for a path outside the
/// captured closure (closed-read discipline), never a silent absence.
type ReadBytes<'a> = dyn Fn(&str) -> Result<Option<Vec<u8>>> + 'a;

fn read_text(read: &ReadBytes<'_>, path: &str) -> Result<Option<String>> {
    read(path)?
        .map(|bytes| String::from_utf8(bytes).context(format!("{path} is not UTF-8")))
        .transpose()
}

fn required_text(read: &ReadBytes<'_>, path: &str) -> Result<String> {
    read_text(read, path)?.ok_or_else(|| anyhow!("required repository file '{path}' is missing"))
}

fn load_config(read: &ReadBytes<'_>) -> Result<JitConfig> {
    let mut config: JitConfig = toml::from_str(
        read_text(read, ".jit/config.toml")?
            .as_deref()
            .unwrap_or(""),
    )
    .context("invalid .jit/config.toml")?;
    let hierarchy_types: Vec<&str> = config
        .type_hierarchy
        .as_ref()
        .map(|hierarchy| hierarchy.types.keys().map(String::as_str).collect())
        .unwrap_or_default();
    config.templates = match read_text(read, ".jit/templates.toml")? {
        Some(content) => {
            crate::templates::TemplateRegistry::from_toml_str(&content, &hierarchy_types)
                .context("invalid .jit/templates.toml")?
        }
        None => crate::templates::TemplateRegistry::empty(),
    };
    config.invariants = match read_text(read, ".jit/invariants.toml")? {
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
    read: &ReadBytes<'_>,
    config: &JitConfig,
    namespaces: &crate::domain::LabelNamespaces,
) -> Result<RuleSet> {
    let Some(content) = read_text(read, ".jit/rules.toml")? else {
        return Ok(crate::repository_state::default_ruleset(namespaces));
    };
    let schemas = RuleSet::schema_requests(&content)?.into_iter().try_fold(
        BTreeMap::new(),
        |mut schemas, request| {
            let path = format!(".jit/{}", request.reference);
            let synthetic_path = PathBuf::from(&path);
            match read(&path) {
                Ok(Some(bytes)) => {
                    schemas.insert(request.reference, bytes);
                }
                Ok(None) if !request.required => {}
                Ok(None) => {
                    return Err(RuleConfigError::SchemaIo {
                        rule: request.rule,
                        path: synthetic_path,
                        source: IoError::new(
                            ErrorKind::NotFound,
                            "schema absent from repository image",
                        ),
                    });
                }
                Err(_) if !request.required => {}
                Err(error) => {
                    return Err(RuleConfigError::SchemaIo {
                        rule: request.rule,
                        path: synthetic_path,
                        source: IoError::other(error.to_string()),
                    });
                }
            }
            Ok(schemas)
        },
    )?;
    let parsed = RuleSet::parse(&content, Some(config), schemas)?;
    Ok(crate::repository_state::reconcile_default_rules_with_config(parsed, namespaces))
}

#[derive(Deserialize)]
struct GatesFile {
    #[serde(default)]
    gates: Vec<crate::declarations::GateDefinition>,
}

fn load_gates(read: &ReadBytes<'_>) -> Result<GateRegistry> {
    let content = read_text(read, ".jit/gates.toml")?.unwrap_or_default();
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

fn load_records(read: &ReadBytes<'_>, image: &RepositoryImage) -> Result<Records> {
    let index: RepositoryIndex = serde_json::from_str(&required_text(read, ".jit/index.json")?)
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
            let issue: Issue = serde_json::from_str(&required_text(read, &path)?)
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
    // The complete `.jit/issues` listing is captured into the image; reconcile the
    // JSON issue files it contains against the index ids exactly as the recursive
    // filesystem walk did, but from the closed listing fingerprint.
    let issues_dir = crate::repository_state::VirtualPath::data("issues")?;
    let listing = image
        .listing_fingerprints()
        .get(&issues_dir)
        .ok_or_else(|| anyhow!("captured image is missing the .jit/issues listing"))?;
    let expected: BTreeSet<String> = index
        .all_ids
        .iter()
        .map(|id| format!("{id}.json"))
        .collect();
    let actual: BTreeSet<String> = listing
        .children()
        .keys()
        .filter(|name| name.ends_with(".json"))
        .cloned()
        .collect();
    if actual != expected {
        return Err(anyhow!(
            "issue files disagree with .jit/index.json (expected {expected:?}, found {actual:?})"
        ));
    }

    let mut event_count = 0;
    if let Some(events) = read_text(read, ".jit/events.jsonl")? {
        event_count = parse_known_events(&events)
            .context("invalid .jit/events.jsonl")?
            .len();
    }
    Ok(Records {
        issues,
        event_count,
    })
}

fn validate_integrity(
    image: &RepositoryImage,
    issues: &[Issue],
    gates: &GateRegistry,
) -> Result<()> {
    let ids: HashSet<&str> = issues.iter().map(|issue| issue.id.as_str()).collect();
    for issue in issues {
        for dependency in &issue.dependencies {
            if !ids.contains(dependency.as_str()) {
                return Err(anyhow!(
                    "Invalid dependency: issue '{}' depends on '{}' which does not exist",
                    issue.id,
                    dependency
                ));
            }
        }
        for gate in &issue.gates_required {
            if !gates.gates.contains_key(gate) {
                return Err(anyhow!(
                    "Gate '{}' required by issue '{}' is not defined in registry",
                    gate,
                    issue.id
                ));
            }
        }
    }

    // Document references resolve against boundary-acquired evidence in the closed
    // image, never live Git or filesystem I/O (plan §2 "Pinned document evidence").
    // A pinned reference is checked against the pinned evidence for its
    // `(commit, path)` request; an unpinned reference is present when its captured
    // working-tree entry exists, falling back to boundary-acquired HEAD evidence.
    // When Git is unavailable the pinned evidence carries a stable unavailable
    // reason, reproducing the typed pinned-read diagnostic without requiring Git.
    for issue in issues {
        for document in &issue.documents {
            if let Some(reference) = document.commit.as_deref() {
                let evidence = image
                    .pinned_evidence()
                    .get(&(reference.to_string(), document.path.clone()));
                match evidence {
                    Some(evidence) if evidence.exists() => {}
                    Some(evidence) => {
                        return Err(anyhow!(
                            "Invalid document reference in issue '{}': pinned document '{}' at commit '{}' is unavailable: {}",
                            issue.id,
                            document.path,
                            reference,
                            evidence.unavailable_reason().unwrap_or("not found")
                        ));
                    }
                    None => {
                        return Err(anyhow!(
                            "Invalid document reference in issue '{}': pinned document '{}' at commit '{}' was not captured",
                            issue.id,
                            document.path,
                            reference
                        ));
                    }
                }
            } else {
                let worktree = crate::repository_state::VirtualPath::worktree(&document.path)?;
                let present = matches!(image.entry(&worktree)?, RepositoryEntry::File { .. });
                if !present {
                    let head = image
                        .pinned_evidence()
                        .get(&("HEAD".to_string(), document.path.clone()));
                    let in_head =
                        head.is_some_and(crate::repository_state::PinnedDocumentEvidence::exists);
                    if !in_head {
                        return Err(anyhow!(
                            "Invalid document reference in issue '{}': file '{}' not found in the working tree or at HEAD",
                            issue.id,
                            document.path
                        ));
                    }
                }
            }
        }
    }
    let refs: Vec<&Issue> = issues.iter().collect();
    let graph = DependencyGraph::new(&refs);
    graph.validate_dag()?;
    if issues.len() > 1 {
        let isolated = graph.get_isolated_nodes();
        if !isolated.is_empty() {
            let isolated_ids = isolated
                .iter()
                .map(|issue| format!("'{}' ({})", issue.short_id(), issue.title))
                .collect::<Vec<_>>()
                .join("\n  ");
            return Err(anyhow!(
                "Found {} isolated issue(s) not connected to the dependency graph:\n  {}\n\
                 Isolated issues have no dependencies and are not dependencies of any other issue.\n\
                 Either add dependencies with 'jit dep add' or delete these issues.",
                isolated.len(),
                isolated_ids
            ));
        }
    }
    for issue in issues {
        let reduced = graph.compute_transitive_reduction(&issue.id);
        let reduced_set: HashSet<&String> = reduced.iter().collect();
        for dependency in &issue.dependencies {
            if !reduced_set.contains(dependency) {
                let path = graph.find_shortest_path(&issue.id, dependency);
                let path = if path.is_empty() {
                    "unknown path".to_string()
                } else {
                    path.iter()
                        .map(|id| &id[..SHORT_ID_LENGTH.min(id.len())])
                        .collect::<Vec<_>>()
                        .join(" → ")
                };
                return Err(anyhow!(
                    "Transitive reduction violation: Issue {} has redundant dependency on {} \
                     (already reachable via: {}). Run 'jit validate --fix' to remove redundant edges.",
                    issue.short_id(),
                    dependency.chars().take(SHORT_ID_LENGTH).collect::<String>(),
                    path
                ));
            }
        }
    }
    Ok(())
}

/// Validate the machine-local claims control plane selected by the repository
/// root. This boundary intentionally sits beside the captured repository image:
/// claims live in `.git/jit`, not in the planned `.jit` byte set, so overlays
/// delegate to the same coordination state without reopening repository storage.
fn validate_machine_local_claims(repository_root: &Path) -> Result<()> {
    if std::env::var("JIT_TEST_MODE").is_err() {
        let index_issues = crate::commands::validate_claims_index_at(repository_root)
            .unwrap_or_else(|error| vec![format!("Failed to validate claims index: {error}")]);
        if !index_issues.is_empty() {
            return Err(anyhow!(
                "Claims index validation failed:\n  {}",
                index_issues.join("\n  ")
            ));
        }
    }
    Ok(())
}

fn collect_rule_findings(
    read: &ReadBytes<'_>,
    issues: &[Issue],
    rules: &RuleSet,
    namespaces: &crate::domain::LabelNamespaces,
    config: &JitConfig,
) -> Result<Vec<ReportedFinding>> {
    let repo_format = config
        .validation
        .as_ref()
        .map_or(Ok(crate::domain::ContentFormat::Markdown), |validation| {
            validation.content_format()
        })?;
    let mut reported = Vec::new();
    for issue in issues {
        let evaluation = crate::validation::evaluate_local(issue, rules, repo_format)?;
        reported.extend(
            evaluation
                .findings()
                .into_iter()
                .map(|finding| ReportedFinding::new(Some(issue.id.clone()), finding)),
        );
    }
    let graph_rules: Vec<_> = rules
        .rules
        .iter()
        .filter(|rule| rule.scope == crate::declarations::rules::RuleScope::Graph)
        .collect();
    let hierarchy = crate::repository_state::hierarchy_config(namespaces);
    let plan_content = resolve_plan_content(read, issues, config)?;
    let graph_findings = crate::validation::graph::evaluate_graph(
        &graph_rules,
        issues,
        &hierarchy,
        repo_format,
        chrono::Utc::now(),
        &plan_content,
    );
    reported.extend(
        graph_findings
            .iter()
            .map(|finding| ReportedFinding::new(finding.issue_id.clone(), &finding.finding)),
    );
    Ok(reported)
}

fn collect_enforcement_drift_findings(
    config: &JitConfig,
    rules: Option<&RuleSet>,
    gates: &GateRegistry,
) -> Vec<ReportedFinding> {
    use crate::validation::drift::{enforcement_drift_tolerant, SourceState};

    let rule_names: Option<BTreeSet<&str>> =
        rules.map(|rules| rules.rules.iter().map(|rule| rule.name.as_str()).collect());
    let gate_keys: BTreeSet<&str> = gates.gates.keys().map(String::as_str).collect();
    enforcement_drift_tolerant(
        &config.invariants.invariants,
        rule_names
            .as_ref()
            .map_or(SourceState::Unloadable, SourceState::Loaded),
        SourceState::Loaded(&gate_keys),
    )
    .into_iter()
    .map(|finding| {
        ReportedFinding::new(
            None,
            &Finding {
                rule: crate::commands::ENFORCEMENT_DRIFT_RULE.to_string(),
                severity: Severity::Error,
                message: finding.message(),
            },
        )
    })
    .collect()
}

fn collect_review_placeholder_findings(gates: &GateRegistry) -> Vec<ReportedFinding> {
    let mut keys: Vec<&str> = gates
        .gates
        .iter()
        .filter_map(|(key, gate)| {
            matches!(gate.checker, Some(GateChecker::ReviewPlaceholder)).then_some(key.as_str())
        })
        .collect();
    keys.sort_unstable();
    if keys.is_empty() {
        Vec::new()
    } else {
        vec![ReportedFinding::new(
            None,
            &Finding {
                rule: crate::commands::REVIEW_PLACEHOLDER_RULE.to_string(),
                severity: Severity::Warn,
                message: format!(
                    "WARNING: passing external-review placeholder still configured for gate(s): {}. Replace each placeholder with a real review checker before relying on these gates.",
                    keys.join(", ")
                ),
            },
        )]
    }
}

fn resolve_plan_content(
    read: &ReadBytes<'_>,
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
        match read_text(read, &path)? {
            Some(plan) => {
                content.insert(issue.id.clone(), plan);
            }
            None if planning.is_none_or(|node| node.state != crate::domain::State::Done) => {}
            None => return Err(anyhow!("required plan document '{path}' is missing")),
        }
    }
    Ok(content)
}

fn collect_item_link_findings(
    read: &ReadBytes<'_>,
    issues: &[Issue],
    config: &JitConfig,
) -> Result<Vec<ReportedFinding>> {
    let kinds = resolve_item_kinds(config.item_kinds.as_ref())?;
    if kinds.is_empty() {
        return Ok(Vec::new());
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
    let mut findings = Vec::new();
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
            if let Some(content) = read_text(read, &descriptor.toml)? {
                registry_items.extend(load_toml_scope_items(kind.name(), descriptor, &content)?);
            }
        } else if let Some(path) = kind.source() {
            if let Some(markdown) = read_text(read, path)? {
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
            let canonical = (|| -> Result<String> {
                let address = if value.starts_with('@') {
                    parse_kind_segmented_address(value)?
                } else {
                    expand_sugar_address(value, &kinds)?
                };
                match address.scope {
                    AddressScope::Project => Ok(format!("@/{}/{}", address.kind, address.self_id)),
                    AddressScope::NamedProject(name) => {
                        let declared = config
                            .project
                            .as_ref()
                            .and_then(|project| project.name.as_ref())
                            .map(crate::config::ProjectName::as_str);
                        if declared != Some(name.as_str()) {
                            return Err(anyhow!(
                                "addresses project '{name}', not the local project"
                            ));
                        }
                        Ok(format!("@/{}/{}", address.kind, address.self_id))
                    }
                    AddressScope::Issue(issue_ref) => {
                        let matches: Vec<&Issue> = issues
                            .iter()
                            .filter(|candidate| candidate.id.starts_with(&issue_ref))
                            .collect();
                        let [owner] = matches.as_slice() else {
                            return Err(anyhow!(
                                "has an unresolved or ambiguous issue scope '{issue_ref}'"
                            ));
                        };
                        Ok(format!(
                            "@/issue/{}/{}/{}",
                            owner.short_id(),
                            address.kind,
                            address.self_id
                        ))
                    }
                }
            })();
            let detail = match canonical {
                Ok(canonical) if addresses.contains(&canonical) => continue,
                Ok(_) => format!("the qualified id '{value}' resolves to no addressable item"),
                Err(error) => error.to_string(),
            };
            findings.push(ReportedFinding::new(
                Some(issue.id.clone()),
                &Finding {
                    rule: crate::commands::DANGLING_LINK_RULE.to_string(),
                    severity: Severity::Error,
                    message: format!(
                        "issue {} has a dangling item link '{label}': {detail}",
                        issue.short_id()
                    ),
                },
            ));
        }
    }
    Ok(findings)
}

fn projected_content(
    read: &ReadBytes<'_>,
    target: &str,
    mode: ProjectionMode,
    rendered: &str,
    begin: &str,
    end: &str,
) -> Result<String> {
    match mode {
        ProjectionMode::SeparateFile => Ok(rendered.to_string()),
        ProjectionMode::Region => {
            let current = read_text(read, target)?
                .ok_or_else(|| anyhow!("projection target '{target}' is missing"))?;
            Ok(splice_region(&current, rendered, begin, end)?)
        }
    }
}

fn validate_projections(
    read: &ReadBytes<'_>,
    config: &JitConfig,
    rules: &RuleSet,
    gates: &GateRegistry,
) -> Result<()> {
    let Some(projections) = config.projection.as_ref() else {
        return Ok(());
    };
    let inputs = ProjectionInputs {
        config,
        rules,
        gates,
    };
    for (name, projection) in projections {
        // Re-render the body through the SAME generic code path `jit project
        // render` uses (binding freshness to the projection code, never a stored
        // mirror), then compare the spliced-in result against the target's bytes.
        let mut render_read = |path: &str| read_text(read, path);
        let (body, _count) = render_projection_body(projection, &inputs, &mut render_read)?;
        let target = require_target(projection, name)?;
        let expected = projected_content(
            read,
            &target,
            projection.mode(),
            &body,
            &projection.region_begin(name),
            &projection.region_end(name),
        )?;
        let actual = read_text(read, &target)?
            .ok_or_else(|| anyhow!("projection target '{target}' is missing"))?;
        if actual != expected {
            return Err(anyhow!("projection '{name}' target '{target}' is stale"));
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

    /// A file-backed executor with the fixture's canonical layout, used to capture
    /// whole-repository validation images through the recovered session.
    fn executor(repo: &tempfile::TempDir) -> crate::commands::CommandExecutor<JsonFileStorage> {
        let data = repo.path().join(".jit");
        let layout = crate::storage::discover_repository_layout(repo.path(), &data).unwrap();
        crate::commands::CommandExecutor::new(JsonFileStorage::new(&data)).with_layout(layout)
    }

    /// Read a repo-relative fixture file's bytes, or `None` when absent.
    fn read_fixture(repo: &tempfile::TempDir, repo_rel: &str) -> Option<Vec<u8>> {
        let path = match repo_rel.strip_prefix(".jit/") {
            Some(rest) => repo.path().join(".jit").join(rest),
            None => repo.path().join(repo_rel),
        };
        std::fs::read(path).ok()
    }

    /// Capture and validate the live fixture repository.
    fn validate_live(
        repo: &tempfile::TempDir,
    ) -> std::result::Result<RepositoryValidationReport, RepositoryValidationFailure> {
        executor(repo).validate_repository_report().unwrap()
    }

    /// Capture and validate the fixture overlaid with proposed final bytes/absence.
    fn validate_overlaid<P: Into<PathBuf>>(
        repo: &tempfile::TempDir,
        changes: impl IntoIterator<Item = (P, Option<Vec<u8>>)>,
    ) -> std::result::Result<RepositoryValidationReport, RepositoryValidationFailure> {
        let overrides = crate::commands::overrides_from_repo_changes(
            changes
                .into_iter()
                .map(|(path, value)| (path.into(), value)),
        )
        .unwrap();
        let image = executor(repo)
            .capture_validation_image_with(&overrides)
            .unwrap();
        validate_repository(&image)
    }

    /// A string-valued overlay change (create/replace with UTF-8 text).
    fn put(path: &'static str, text: &str) -> (&'static str, Option<Vec<u8>>) {
        (path, Some(text.as_bytes().to_vec()))
    }

    #[test]
    fn test_live_and_identity_overlay_preserve_validation_results() {
        let repo = fixture();
        let plain = validate_live(&repo).unwrap();
        let planned = validate_overlaid::<&str>(&repo, []).unwrap();
        assert_eq!(plain, planned);
        assert_eq!(plain.passes.len(), 8);
    }

    #[test]
    fn test_overlay_config_ignores_disagreeing_live_bytes() {
        let repo = fixture();
        std::fs::write(repo.path().join(".jit/config.toml"), "not toml = [").unwrap();
        assert!(validate_overlaid(
            &repo,
            [put(".jit/config.toml", "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n")],
        )
        .is_ok());
    }

    #[test]
    fn test_overlay_templates_ignore_disagreeing_live_bytes() {
        let repo = fixture();
        std::fs::write(repo.path().join(".jit/templates.toml"), "not toml = [").unwrap();
        assert!(validate_overlaid(&repo, [put(".jit/templates.toml", "templates = []\n")]).is_ok());
        assert!(validate_live(&repo).is_err());
    }

    #[test]
    fn test_overlay_rules_ignore_disagreeing_live_bytes() {
        let repo = fixture();
        std::fs::write(repo.path().join(".jit/rules.toml"), "not toml = [").unwrap();
        assert!(validate_overlaid(&repo, [put(".jit/rules.toml", "")]).is_ok());
        let live = validate_live(&repo).unwrap();
        assert_eq!(live.rule_report.findings[0].rule, "rules-file");
    }

    #[test]
    fn test_overlay_rules_and_referenced_schema_ignore_disagreeing_live_schema() {
        let repo = fixture();
        let rules = "[[rules]]\nname = \"planned-schema\"\nseverity = \"error\"\nassert = { json-schema = \"schemas/planned.json\" }\n";
        std::fs::create_dir_all(repo.path().join(".jit/schemas")).unwrap();
        std::fs::write(repo.path().join(".jit/rules.toml"), rules).unwrap();
        std::fs::write(repo.path().join(".jit/schemas/planned.json"), "not json").unwrap();
        assert!(validate_overlaid(
            &repo,
            [
                put(".jit/rules.toml", rules),
                put(".jit/schemas/planned.json", "{}"),
            ],
        )
        .is_ok());
        let live_report = validate_live(&repo).unwrap();
        assert!(live_report.rule_report.has_errors());
        assert_eq!(live_report.rule_report.findings[0].rule, "rules-file");
    }

    #[test]
    fn test_overlay_gate_registry_ignores_disagreeing_live_registry() {
        let repo = fixture();
        std::fs::write(repo.path().join(".jit/gates.toml"), "not toml = [").unwrap();
        assert!(validate_overlaid(&repo, [put(".jit/gates.toml", "")]).is_ok());
    }

    #[test]
    fn test_overlay_index_events_and_issue_records_ignore_disagreeing_live_bytes() {
        let repo = fixture();
        let index = read_fixture(&repo, ".jit/index.json").unwrap();
        let events = read_fixture(&repo, ".jit/events.jsonl").unwrap_or_default();
        std::fs::write(repo.path().join(".jit/index.json"), "not json").unwrap();
        std::fs::write(repo.path().join(".jit/events.jsonl"), "not jsonl").unwrap();
        assert!(validate_overlaid(
            &repo,
            [
                (".jit/index.json", Some(index)),
                (".jit/events.jsonl", Some(events)),
            ],
        )
        .is_ok());
    }

    #[test]
    fn test_overlay_records_dag_and_gate_integrity_judge_planned_bytes() {
        let repo = fixture();
        let index_bytes = read_fixture(&repo, ".jit/index.json").unwrap();
        let id = serde_json::from_slice::<RepositoryIndex>(&index_bytes)
            .unwrap()
            .all_ids
            .remove(0);
        let mut issue: Issue = serde_json::from_slice(
            &read_fixture(&repo, &format!(".jit/issues/{id}.json")).unwrap(),
        )
        .unwrap();
        issue.dependencies = vec!["missing".to_string()];
        issue.gates_required = vec!["planned".to_string()];
        let bad = serde_json::to_vec_pretty(&issue).unwrap();
        let error = format!(
            "{:#}",
            validate_overlaid(
                &repo,
                [
                    (format!(".jit/issues/{id}.json"), Some(bad)),
                    (
                        ".jit/gates.toml".to_string(),
                        Some(b"[[gates]]\nkey = \"planned\"\ntitle = \"p\"\ndescription = \"\"\nstage = \"postcheck\"\nmode = \"manual\"\n".to_vec()),
                    ),
                ],
            )
            .unwrap_err()
        );
        assert!(
            error.contains("repository-integrity") && error.contains("missing"),
            "{error}"
        );
    }

    #[test]
    fn test_overlay_namespace_and_hierarchy_judge_planned_config() {
        let repo = fixture();
        let report = validate_overlaid(
            &repo,
            [put(
                ".jit/config.toml",
                "[type_hierarchy.types]\nepic = 2\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
            )],
        )
        .unwrap();
        assert!(report.rule_report.has_errors());
        assert!(report
            .rule_report
            .findings
            .iter()
            .any(|finding| finding.rule == "type-hierarchy-known"));
    }

    #[test]
    fn test_repository_view_keeps_non_enforced_rule_error_reportable() {
        let repo = fixture();
        let report = validate_overlaid(
            &repo,
            [put(
                ".jit/rules.toml",
                "[[rules]]\nname = \"task-needs-req\"\nwhen = { type = \"task\" }\n\
                 severity = \"error\"\nenforce = false\n\
                 assert = { require-label = { label = \"req:*\", min = 1 } }\n",
            )],
        )
        .unwrap();
        assert_eq!(report.rule_report.error_count(), 1);
        assert_eq!(report.rule_report.findings[0].rule, "task-needs-req");
    }

    #[test]
    fn test_repository_failure_retains_partial_rule_report() {
        let repo = fixture();
        let store = JsonFileStorage::new(repo.path().join(".jit"));
        let mut issue = store.list_issues().unwrap().remove(0);
        issue.dependencies.push("nonexistent".to_string());
        let rules = "[[rules]]\nname = \"task-needs-req\"\nwhen = { type = \"task\" }\n\
                     severity = \"error\"\nenforce = false\n\
                     assert = { require-label = { label = \"req:*\", min = 1 } }\n";
        let live = validate_live(&repo).unwrap();
        assert!(!live.rule_report.has_errors());
        let failure = validate_overlaid(
            &repo,
            [
                (
                    ".jit/rules.toml".to_string(),
                    Some(rules.as_bytes().to_vec()),
                ),
                (
                    format!(".jit/issues/{}.json", issue.id),
                    Some(serde_json::to_vec_pretty(&issue).unwrap()),
                ),
            ],
        )
        .unwrap_err();
        assert!(failure.to_string().contains("does not exist"), "{failure}");
        assert_eq!(failure.report().rule_report.error_count(), 1);
        assert_eq!(
            failure.report().rule_report.findings[0].rule,
            "task-needs-req"
        );
    }

    #[test]
    fn test_overlay_item_links_judge_planned_registry_deletion() {
        let repo = fixture();
        let config = "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n[namespaces.enforces]\ndescription = \"Item link\"\nunique = false\n\
            [item_kinds.invariant]\nsection = \"success_criteria\"\nid-pattern = \"[a-z-]+\"\nmarkers = []\nlink-namespaces = [\"enforces\"]\nscope = \"project\"\nsource-of-truth = \"registry-first\"\nsource = { toml = \".jit/invariants.toml\", table = \"invariants\", id-field = \"id\", text-field = \"statement\" }\n\
            ";
        let invariants = "[[invariants]]\nid = \"planned\"\nstatement = \"planned bytes win\"\nkind = \"advisory\"\n";
        std::fs::write(repo.path().join(".jit/config.toml"), config).unwrap();
        std::fs::write(repo.path().join(".jit/invariants.toml"), invariants).unwrap();
        let index: RepositoryIndex =
            serde_json::from_slice(&read_fixture(&repo, ".jit/index.json").unwrap()).unwrap();
        let id = &index.all_ids[0];
        let mut issue: Issue = serde_json::from_slice(
            &read_fixture(&repo, &format!(".jit/issues/{id}.json")).unwrap(),
        )
        .unwrap();
        issue
            .labels
            .push("enforces:@/invariant/planned".to_string());
        std::fs::write(
            repo.path().join(format!(".jit/issues/{id}.json")),
            serde_json::to_vec_pretty(&issue).unwrap(),
        )
        .unwrap();

        let live = validate_live(&repo).unwrap();
        assert!(!live.rule_report.has_errors(), "{:?}", live.rule_report);
        let planned = validate_overlaid(&repo, [(".jit/invariants.toml", None)]).unwrap();
        assert!(planned
            .rule_report
            .findings
            .iter()
            .any(|finding| finding.rule == crate::commands::DANGLING_LINK_RULE));
    }

    #[test]
    fn test_overlay_projection_rejects_stale_planned_target_despite_live_state() {
        let repo = fixture();
        let config = "\
[type_hierarchy.types]
task = 4
[namespaces.type]
description = \"Issue type\"
unique = true
[item_kinds.invariant]
section = \"success_criteria\"
id-pattern = \"[a-z][a-z0-9-]*\"
markers = []
link-namespaces = [\"enforces\"]
scope = \"project\"
source = { toml = \".jit/invariants.toml\", table = \"invariants\", id-field = \"id\", text-field = \"statement\" }
source-of-truth = \"registry-first\"
[projection.invariants]
kind = \"invariant\"
target = \"INVARIANTS.md\"
mode = \"separate-file\"
style = \"full\"
";
        let invariants = "[[invariants]]\nid = \"planned\"\nstatement = \"planned bytes win\"\nkind = \"advisory\"\n";
        let error = validate_overlaid(
            &repo,
            [
                put(".jit/config.toml", config),
                put(".jit/invariants.toml", invariants),
                put("INVARIANTS.md", "stale planned projection\n"),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("projections validation pass"), "{error}");
    }
}
