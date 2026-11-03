//! Read-only, byte-exact repository validation.
//!
//! Declaration, record, rule, and materialization checks read a closed
//! [`RepositoryImage`]. Full validation supplies the document-reference and
//! record-fidelity verdicts separately, since their evidence lies outside that
//! image. No pass performs live filesystem or Git I/O.

use crate::config::JitConfig;
use crate::declarations::rules::{RuleSet, Severity};
use crate::declarations::GateChecker;
use crate::declarations::GateRegistry;
use crate::document::content_parser_for;
use crate::domain::identity::GateKey;
use crate::domain::identity::IssueId;
use crate::domain::item::{
    expand_sugar_address, index_items, index_project_sources, is_qualified_reference,
    load_toml_scope_items, parse_kind_segmented_address, resolve_item_kinds,
    unqualified_link_message, AddressScope, ProjectSource, RawScopeItem,
};
use crate::domain::{Issue, ReadinessCorrection};
use crate::graph::DependencyGraph;
use crate::repository_state::{captured_issue_ids, RepositoryImage, RepositoryIndex, VirtualPath};
use crate::validation::engine::Finding;
use crate::validation::report::{ReportedFinding, RuleReport};
use anyhow::{anyhow, Context, Result};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

/// One named stage in whole-repository validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepositoryValidationPass {
    /// `config.toml`, `templates.toml`, and `invariants.toml` parse and agree.
    EffectiveConfig,
    /// `rules.toml` and referenced `schemas/*.json` are inspected from the view.
    RulesAndSchemas,
    /// `gates.toml` parses and gate identities are unique.
    Gates,
    /// `index.json` and issue JSON parse and agree on embedded ids.
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
    /// Declarative and built-in semantic findings produced from the same view.
    pub rule_report: RuleReport,
}

/// Validation read the repository and found it wrong: a verdict on its content,
/// where every other structural failure means it could not load or read what it
/// needed.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct RepositoryJudgedInvalid(pub(crate) String);

fn judged_invalid(error: anyhow::Error) -> anyhow::Error {
    RepositoryJudgedInvalid(format!("{error:#}")).into()
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

    pub(crate) fn declaration(error: anyhow::Error) -> Self {
        Self::new(
            error.context("declaration validation pass"),
            RepositoryValidationReport {
                passes: Vec::new(),
                issue_count: 0,
                rule_report: RuleReport::default(),
            },
        )
    }

    pub(crate) fn materialization(error: anyhow::Error) -> Self {
        Self::new(
            error.context("derived-materialization validation pass"),
            RepositoryValidationReport {
                passes: Vec::new(),
                issue_count: 0,
                rule_report: RuleReport::default(),
            },
        )
    }

    /// Whether validation judged the repository wrong rather than being unable to judge.
    pub fn is_judged_invalid(&self) -> bool {
        self.error
            .downcast_ref::<RepositoryJudgedInvalid>()
            .is_some()
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
    let vpath = image.layout().classify_repository_relative(repo_rel)?;
    Ok(image.file_bytes(&vpath)?.map(<[u8]>::to_vec))
}

/// Validate declarations, records, rules, and materializations in `image`.
///
/// Mutations use this pipeline without auditing unrelated document references;
/// full validation supplies that pass's separately captured verdict through
/// [`validate_repository_with_materializations`].
///
/// The image is the single closed evidence source: a
/// live capture validates the working tree, and an overlay image
/// ([`apply_overlay`](crate::repository_state::apply_overlay)) validates a proposed
/// final state. Every pass reads only image-projected content — no live filesystem
/// or Git I/O — so a path outside the captured closure fails typed rather than
/// reading through.
pub fn validate_repository(
    image: &RepositoryImage,
) -> std::result::Result<RepositoryValidationReport, RepositoryValidationFailure> {
    let declarations = crate::repository_state::validation_declarations_from_image(image)
        .map_err(|error| RepositoryValidationFailure::declaration(error.into()))?;
    let expected = declarations
        .rules_loaded()
        .then(|| {
            crate::repository_state::derive_materialization(
                image,
                crate::repository_state::MaterializationRequest::RepairDerivedState {
                    declarations: declarations.borrowed(),
                },
            )
        })
        .transpose()
        .map_err(|error| {
            RepositoryValidationFailure::new(
                error.into(),
                RepositoryValidationReport {
                    passes: Vec::new(),
                    issue_count: 0,
                    rule_report: RuleReport::default(),
                },
            )
        })?;
    validate_repository_with_materializations(
        image,
        &declarations,
        expected.as_ref(),
        Ok(()),
        Ok(()),
    )
}

pub(crate) fn validate_repository_with_materializations(
    image: &RepositoryImage,
    declarations: &crate::repository_state::CapturedRepositoryDeclarations,
    expected: Option<&crate::repository_state::MaterializationPlan>,
    document_references: Result<()>,
    record_fidelity: Result<()>,
) -> std::result::Result<RepositoryValidationReport, RepositoryValidationFailure> {
    let read = move |path: &str| image_read(image, path);
    let read: &ReadBytes<'_> = &read;
    let mut passes = Vec::new();
    let config = declarations.config().clone();
    if let Err(error) = config.validate_item_kinds() {
        return Err(RepositoryValidationFailure::new(
            anyhow!(error).context("effective-config validation pass"),
            RepositoryValidationReport {
                passes,
                issue_count: 0,
                rule_report: RuleReport::default(),
            },
        ));
    }
    passes.push(RepositoryValidationPass::EffectiveConfig);

    let namespaces = crate::config_manager::namespaces_from_config(&config);
    let rules = declarations.rules().clone();
    let rules_loaded = declarations.rules_loaded();
    let mut findings = Vec::new();
    if rules_loaded {
        passes.push(RepositoryValidationPass::RulesAndSchemas);
    } else if let Some(error) = declarations.rules_load_error() {
        findings.push(ReportedFinding::new(
            None,
            &Finding {
                rule: "rules-file".to_string(),
                severity: Severity::Error,
                message: format!("config error: {error:#}"),
            },
        ));
    }

    let mut structural_error = None;
    let gates = declarations.gates().clone();
    passes.push(RepositoryValidationPass::Gates);

    let issues = match load_records(read, image).context("records validation pass") {
        Ok(issues) => {
            passes.push(RepositoryValidationPass::Records);
            Some(issues)
        }
        Err(error) => {
            if structural_error.is_none() {
                structural_error = Some(error);
            }
            None
        }
    };

    if structural_error.is_none() {
        structural_error = record_fidelity
            .context("record-fidelity validation pass")
            .err();
    }

    if let Some(issues) = &issues {
        let integrity_error = validate_integrity(issues, &gates, document_references)
            .map_err(judged_invalid)
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

    let mut namespace_and_hierarchy_complete = rules_loaded;
    if rules_loaded {
        if let Some(issues) = &issues {
            match collect_rule_findings(read, issues, &rules, &namespaces, &config)
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
    findings.extend(collect_enforcement_drift_findings(
        &config,
        rules_loaded.then_some(&rules),
        &gates,
    ));
    findings.extend(collect_review_placeholder_findings(&gates));
    if namespace_and_hierarchy_complete {
        passes.push(RepositoryValidationPass::NamespaceAndHierarchy);
    }

    if let Some(issues) = &issues {
        let credit_namespaces = if rules_loaded {
            crate::validation::graph::coverage_credit_namespaces(&rules.rules)
        } else {
            HashSet::new()
        };
        match collect_item_link_findings(read, issues, &config, &credit_namespaces)
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

    if let Some(expected) = expected {
        match validate_materializations(image, expected)
            .context("derived-materialization validation pass")
        {
            Ok(()) => passes.push(RepositoryValidationPass::Projections),
            Err(error) => {
                if structural_error.is_none() {
                    structural_error = Some(error);
                }
            }
        }
    }

    let report = RepositoryValidationReport {
        passes,
        issue_count: issues.as_ref().map_or(0, Vec::len),
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

fn load_records(read: &ReadBytes<'_>, image: &RepositoryImage) -> Result<Vec<Issue>> {
    let index_path = VirtualPath::INDEX.repository_relative();
    RepositoryIndex::parse(
        &read(&index_path)?
            .ok_or_else(|| anyhow!("required repository file '{index_path}' is missing"))?,
    )
    .context(RepositoryJudgedInvalid(format!("invalid {index_path}")))?;
    captured_issue_ids(image)?
        .iter()
        .map(|id| {
            let path = VirtualPath::issue_record(id).repository_relative();
            let issue: Issue = serde_json::from_str(&required_text(read, &path)?)
                .with_context(|| RepositoryJudgedInvalid(format!("invalid {path}")))?;
            if &issue.id != id {
                return Err(RepositoryJudgedInvalid(format!(
                    "issue file '{path}' contains id '{}' instead of '{id}'",
                    issue.id
                ))
                .into());
            }
            Ok(issue)
        })
        .collect()
}

fn validate_integrity(
    issues: &[Issue],
    gates: &GateRegistry,
    document_references: Result<()>,
) -> Result<()> {
    let ids: HashSet<&IssueId> = issues.iter().map(|issue| &issue.id).collect();
    for issue in issues {
        for dependency in &issue.dependencies {
            if !ids.contains(dependency) {
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

    document_references?;
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
        let reduced_set: HashSet<_> = reduced.iter().map(|node| &node.id).collect();
        for dependency in &issue.dependencies {
            if !reduced_set.contains(dependency) {
                let path = graph.find_shortest_path(&issue.id, dependency);
                let path = if path.is_empty() {
                    "unknown path".to_string()
                } else {
                    path.iter()
                        .map(|node| node.short_id())
                        .collect::<Vec<_>>()
                        .join(" → ")
                };
                return Err(anyhow!(
                    "Transitive reduction violation: Issue {} has redundant dependency on {} \
                     (already reachable via: {}). Run 'jit validate --fix' to remove redundant edges.",
                    issue.short_id(),
                    dependency.short_id(),
                    path
                ));
            }
        }
    }

    // Stored readiness must agree with the readiness the graph derives
    // (`@/invariant/derived-state-coherence`): a Ready issue with unmet
    // dependencies tells an agent it may start work the graph withholds, and an
    // unblocked Backlog issue never becomes available.
    let resolved = crate::domain::queries::build_issue_map(issues);
    let incoherent = issues
        .iter()
        .filter_map(|issue| {
            issue
                .derive_readiness_correction(&resolved)
                .map(|correction| {
                    let detail = match correction {
                        ReadinessCorrection::Promote => {
                            "is Backlog with no unmet dependencies".to_string()
                        }
                        ReadinessCorrection::Demote => {
                            let unmet =
                                crate::domain::queries::unmet_dependencies(issue, &resolved)
                                    .into_iter()
                                    .map(|dependency| dependency.short_id())
                                    .collect::<Vec<_>>()
                                    .join(", ");
                            format!("is Ready but waits on: {unmet}")
                        }
                    };
                    format!("'{}' ({}) {detail}", issue.short_id(), issue.title)
                })
        })
        .collect::<Vec<_>>();
    if !incoherent.is_empty() {
        return Err(anyhow!(
            "Found {} issue(s) whose stored readiness disagrees with the dependency graph:\n  {}\n\
             Run 'jit validate --fix' to move them to the state their dependencies imply.",
            incoherent.len(),
            incoherent.join("\n  ")
        ));
    }
    Ok(())
}

/// Validate the machine-local claims control plane selected by the repository
/// root. This boundary intentionally sits beside the captured repository image:
/// claims live in `.git/jit`, not in the planned `.jit` byte set, so overlays
/// delegate to the same coordination state without reopening repository storage.
fn validate_machine_local_claims(repository_root: &Path) -> Result<()> {
    if !crate::test_mode::is_enabled() {
        let index_issues = crate::commands::validate_claims_index_at(repository_root)
            .context("Failed to validate claims index")?;
        if !index_issues.is_empty() {
            return Err(RepositoryJudgedInvalid(format!(
                "Claims index validation failed:\n  {}",
                index_issues.join("\n  ")
            ))
            .into());
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
    let repo_format = config.content_format();
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
    let hierarchy = crate::domain::type_taxonomy::HierarchyConfig::from_declarations(namespaces)?;
    let plan_content = plan_content(read, issues, issues, &config.templates)?;
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
    use crate::validation::drift::{enforcement_drift, SourceState};

    let rule_names: Option<BTreeSet<&str>> =
        rules.map(|rules| rules.rules.iter().map(|rule| rule.name.as_str()).collect());
    let gate_keys: BTreeSet<GateKey> = gates.gates.keys().cloned().collect();
    enforcement_drift(
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
    let mut keys: Vec<&GateKey> = gates
        .gates
        .iter()
        .filter_map(|(key, gate)| {
            matches!(gate.checker, Some(GateChecker::ReviewPlaceholder)).then_some(key)
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
                    keys.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
                ),
            },
        )]
    }
}

/// Plan content of each container in `emit_for` with a linked plan document,
/// resolving planning nodes over `all_issues`. An absent plan is omitted until
/// its planning node is `done`, then it is an error.
pub(crate) fn plan_content(
    read: &ReadBytes<'_>,
    all_issues: &[Issue],
    emit_for: &[Issue],
    templates: &crate::templates::TemplateRegistry,
) -> Result<HashMap<IssueId, String>> {
    let by_id = all_issues.iter().map(|issue| (&issue.id, issue)).collect();
    emit_for
        .iter()
        .filter_map(|issue| {
            crate::templates::plan_document_ref(issue, &by_id, templates).map(|plan| (issue, plan))
        })
        .map(|(issue, plan)| {
            Ok(match read_text(read, &plan.path)? {
                Some(text) => Some((issue.id.clone(), text)),
                None if plan.planning_done => {
                    return Err(anyhow!("required plan document '{}' is missing", plan.path))
                }
                None => None,
            })
        })
        .filter_map(Result::transpose)
        .collect()
}

fn collect_item_link_findings(
    read: &ReadBytes<'_>,
    issues: &[Issue],
    config: &JitConfig,
    credit_namespaces: &HashSet<String>,
) -> Result<Vec<ReportedFinding>> {
    let kinds = resolve_item_kinds(config.item_kinds.as_ref())?;
    if kinds.is_empty() && credit_namespaces.is_empty() {
        return Ok(Vec::new());
    }
    let repo_format = config.content_format();
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
            if let Some(message) =
                unqualified_link_message(&issue.id, namespace, value, &kinds, credit_namespaces)
            {
                findings.push(ReportedFinding::new(
                    Some(issue.id.clone()),
                    &Finding {
                        rule: crate::commands::DANGLING_LINK_RULE.to_string(),
                        severity: Severity::Error,
                        message,
                    },
                ));
                continue;
            }
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
                        let owner_id = crate::storage::resolve_issue_id_among(&issue_ref, issues)?;
                        let owner = issues
                            .iter()
                            .find(|issue| issue.id == owner_id)
                            .ok_or_else(|| anyhow!("missing issue '{owner_id}'"))?;
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

fn validate_materializations(
    image: &RepositoryImage,
    expected: &crate::repository_state::MaterializationPlan,
) -> Result<()> {
    let drift = crate::repository_state::compare_materializations(image, expected)?;
    if !drift.is_empty() {
        let details = drift
            .iter()
            .map(|finding| {
                let kind = match &finding.kind {
                    crate::repository_state::MaterializationDriftKind::Missing => "missing",
                    crate::repository_state::MaterializationDriftKind::Stale => "stale",
                    crate::repository_state::MaterializationDriftKind::Unexpected => "unexpected",
                };
                format!("- {}: {kind}", finding.path.repository_relative())
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Err(RepositoryJudgedInvalid(format!(
            "derived-state drift (repair with `jit validate --fix`):\n{details}"
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{IssueStore, JsonFileStorage};
    use std::path::PathBuf;

    fn seed_fixture_issue(
        executor: &mut crate::commands::CommandExecutor<JsonFileStorage>,
        title: &str,
        description: &str,
        labels: Vec<String>,
    ) {
        executor
            .issues()
            .create_issue(
                title.to_string(),
                description.to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                labels,
                None,
                None,
                false,
            )
            .unwrap();
    }

    #[test]
    fn test_validate_repository_preserves_invalid_hierarchy_error() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let config = crate::test_taxonomy::invalid_hierarchy_config_fragment();
        std::fs::write(
            data_path(&executor, &VirtualPath::CONFIG),
            config.as_bytes(),
        )
        .unwrap();

        let failure = executor
            .validation()
            .validate_repository_report()
            .unwrap()
            .unwrap_err();
        let (error, _report) = failure.into_parts();

        assert_eq!(
            error.downcast_ref::<crate::domain::type_taxonomy::ConfigError>(),
            Some(&crate::domain::type_taxonomy::ConfigError::InvalidLevel(0))
        );
    }

    fn data_path(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        path: &VirtualPath,
    ) -> PathBuf {
        executor.require_layout().unwrap().resolve(path).unwrap()
    }

    fn issue_path(id: &IssueId) -> VirtualPath {
        VirtualPath::issue_record(id)
    }

    /// Read a repo-relative fixture file's bytes, or `None` when absent.
    fn read_fixture(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        repo_rel: &str,
    ) -> Option<Vec<u8>> {
        let layout = executor.require_layout().unwrap();
        let path = layout
            .resolve(&layout.classify_repository_relative(repo_rel).unwrap())
            .unwrap();
        std::fs::read(path).ok()
    }

    /// Capture and validate the live fixture repository.
    fn validate_live(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
    ) -> std::result::Result<RepositoryValidationReport, RepositoryValidationFailure> {
        executor.validation().validate_repository_report().unwrap()
    }

    /// Capture and validate the fixture overlaid with proposed final bytes/absence.
    fn validate_overlaid<P: Into<PathBuf>>(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        changes: impl IntoIterator<Item = (P, Option<Vec<u8>>)>,
    ) -> std::result::Result<RepositoryValidationReport, RepositoryValidationFailure> {
        let overrides = crate::commands::overrides_from_repo_changes(
            &executor.require_layout().unwrap(),
            changes
                .into_iter()
                .map(|(path, value)| (path.into(), value)),
        )
        .unwrap();
        let image = executor.capture_validation_image_with(&overrides).unwrap();
        validate_repository(&image)
    }

    /// Validate a semantic proposal together with the exact coupled derived-state
    /// writes the canonical materializer would publish for that proposal.
    fn validate_materialized_overlaid<P: Into<PathBuf>>(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        changes: impl IntoIterator<Item = (P, Option<Vec<u8>>)>,
    ) -> std::result::Result<RepositoryValidationReport, RepositoryValidationFailure> {
        let layout = executor.require_layout().unwrap();
        let mut overrides = crate::commands::overrides_from_repo_changes(
            &layout,
            changes
                .into_iter()
                .map(|(path, value)| (path.into(), value)),
        )
        .unwrap();
        let proposed = executor.capture_validation_image_with(&overrides).unwrap();
        let declarations = crate::repository_state::declarations_from_image(&proposed).unwrap();
        let plan = crate::repository_state::derive_materialization(
            &proposed,
            crate::repository_state::MaterializationRequest::RepairDerivedState {
                declarations: declarations.borrowed(),
            },
        )
        .unwrap();
        overrides.extend(crate::commands::validation_overlay(plan.image(), plan.delta()).unwrap());
        let image = executor.capture_validation_image_with(&overrides).unwrap();
        validate_repository(&image)
    }

    /// A string-valued overlay change (create/replace with UTF-8 text).
    fn put(path: impl Into<String>, text: &str) -> (String, Option<Vec<u8>>) {
        (path.into(), Some(text.as_bytes().to_vec()))
    }

    #[test]
    fn test_live_and_identity_overlay_preserve_validation_results() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let plain = validate_live(&executor).unwrap();
        let planned = validate_overlaid::<&str>(&executor, []).unwrap();
        assert_eq!(plain, planned);
        assert_eq!(plain.passes.len(), 8);
    }

    fn seed_attested_gate_run(
        executor: &mut crate::commands::CommandExecutor<JsonFileStorage>,
    ) -> crate::domain::identity::GateRunId {
        seed_fixture_issue(
            executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let key: GateKey = "attested".parse().unwrap();
        executor
            .gates()
            .define_gate(
                &key,
                "Attested".to_string(),
                String::new(),
                crate::declarations::GateStage::Postcheck,
                crate::declarations::GateMode::Manual,
                None,
                100,
            )
            .unwrap();
        let issue = executor.storage().list_issues().unwrap().remove(0);
        executor.gates().add_gate(&issue.id, &key).unwrap();
        executor
            .gates()
            .pass_gate(&issue.id, &key, Some("human:test".to_string()), false)
            .unwrap();
        std::fs::read_dir(data_path(executor, &VirtualPath::GATE_RUNS))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .as_ref()
            .try_into()
            .unwrap()
    }

    #[test]
    fn test_validate_live_accepts_every_record_the_writers_published() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_attested_gate_run(&mut executor);

        validate_live(&executor).unwrap();
    }

    #[test]
    fn test_validate_live_reports_a_gate_run_that_does_not_reproduce() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        let run_id = seed_attested_gate_run(&mut executor);
        let path = VirtualPath::gate_run_record(&run_id);
        let stored = read_fixture(&executor, &path.repository_relative()).unwrap();
        let mut raw: serde_json::Value = serde_json::from_slice(&stored).unwrap();
        raw["retired_key"] = serde_json::Value::Bool(true);
        raw["origin"] = serde_json::json!({ "kind": "not-a-recorded-origin" });
        std::fs::write(
            data_path(&executor, &path),
            serde_json::to_vec(&raw).unwrap(),
        )
        .unwrap();

        let error = validate_live(&executor).unwrap_err().to_string();

        assert!(
            error.contains("record-fidelity") && error.contains(run_id.as_str()),
            "{error}"
        );
    }

    #[test]
    fn test_overlay_validation_does_not_read_gate_run_records() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        let run_id = seed_attested_gate_run(&mut executor);
        std::fs::write(
            data_path(&executor, &VirtualPath::gate_run_record(&run_id)),
            "not json",
        )
        .unwrap();

        validate_overlaid::<&str>(&executor, []).unwrap();
    }

    #[test]
    fn test_unloadable_rules_preserve_rules_file_and_enforcement_drift_findings() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        std::fs::write(
            data_path(&executor, &VirtualPath::RULES),
            "[[rules]]\nname = \"bad-rule\"\nseverity = \"error\"\n\
             assert = { this-is-not-a-valid-kind = { foo = 1 } }\n",
        )
        .unwrap();
        std::fs::write(
            data_path(&executor, &VirtualPath::INVARIANTS),
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\nkind = \"enforced\"\n\
             enforced-by = \"@/rule/bad-rule\"\n",
        )
        .unwrap();

        let report = validate_live(&executor).unwrap();
        assert!(report.rule_report.findings.iter().any(
            |finding| finding.rule == "rules-file" && finding.message.contains("config error")
        ));
        assert!(report.rule_report.findings.iter().any(|finding| {
            finding.rule == crate::commands::ENFORCEMENT_DRIFT_RULE
                && finding.message.contains("declared-but-unenforced")
                && finding.message.contains("failed to load")
        }));
        assert!(!report
            .passes
            .contains(&RepositoryValidationPass::Projections));
    }

    #[test]
    fn test_overlay_config_ignores_disagreeing_live_bytes() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        std::fs::write(data_path(&executor, &VirtualPath::CONFIG), "not toml = [").unwrap();
        assert!(validate_overlaid(
            &executor,
            [put(VirtualPath::CONFIG.repository_relative(), "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n")],
        )
        .is_ok());
    }

    #[test]
    fn test_overlay_templates_ignore_disagreeing_live_bytes() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        std::fs::write(
            data_path(&executor, &VirtualPath::TEMPLATES),
            "not toml = [",
        )
        .unwrap();
        assert!(validate_overlaid(
            &executor,
            [put(
                VirtualPath::TEMPLATES.repository_relative(),
                "templates = []\n"
            )]
        )
        .is_ok());
        assert!(validate_live(&executor).is_err());
    }

    #[test]
    fn test_overlay_rules_ignore_disagreeing_live_bytes() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let rules = read_fixture(&executor, &VirtualPath::RULES.repository_relative()).unwrap();
        std::fs::write(data_path(&executor, &VirtualPath::RULES), "not toml = [").unwrap();
        assert!(validate_overlaid(
            &executor,
            [(VirtualPath::RULES.repository_relative(), Some(rules))]
        )
        .is_ok());
        assert!(validate_live(&executor)
            .unwrap()
            .rule_report
            .findings
            .iter()
            .any(|finding| finding.rule == "rules-file"));
    }

    #[test]
    fn test_overlay_rules_and_referenced_schema_ignore_disagreeing_live_schema() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let schema_path = VirtualPath::schema("planned.json").unwrap();
        let rules = format!(
            "[[rules]]\nname = \"planned-schema\"\nseverity = \"error\"\nassert = {{ json-schema = {:?} }}\n",
            schema_path.relative().as_str(),
        );
        std::fs::create_dir_all(data_path(&executor, &VirtualPath::SCHEMAS)).unwrap();
        std::fs::write(data_path(&executor, &VirtualPath::RULES), &rules).unwrap();
        std::fs::write(data_path(&executor, &schema_path), "not json").unwrap();
        assert!(validate_materialized_overlaid(
            &executor,
            [
                put(VirtualPath::RULES.repository_relative(), &rules),
                put(schema_path.repository_relative(), "{}"),
            ],
        )
        .is_ok());
        assert!(validate_live(&executor)
            .unwrap()
            .rule_report
            .findings
            .iter()
            .any(|finding| finding.rule == "rules-file"
                && finding.message.contains("not valid JSON")));
    }

    #[test]
    fn test_overlay_gate_registry_ignores_disagreeing_live_registry() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        std::fs::write(data_path(&executor, &VirtualPath::GATES), "not toml = [").unwrap();
        assert!(validate_overlaid(
            &executor,
            [put(VirtualPath::GATES.repository_relative(), "")]
        )
        .is_ok());
    }

    #[test]
    fn test_overlay_index_ignores_disagreeing_live_bytes() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let index = read_fixture(&executor, &VirtualPath::INDEX.repository_relative()).unwrap();
        std::fs::write(data_path(&executor, &VirtualPath::INDEX), "not json").unwrap();
        assert!(validate_overlaid(
            &executor,
            [(VirtualPath::INDEX.repository_relative(), Some(index))],
        )
        .is_ok());
    }

    #[test]
    fn test_validate_repository_passes_records_over_an_unparseable_event_log() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );

        let report = validate_overlaid(
            &executor,
            [put(
                VirtualPath::EVENTS.repository_relative(),
                "not jsonl\n",
            )],
        )
        .unwrap();

        assert!(report.passes.contains(&RepositoryValidationPass::Records));
    }

    #[test]
    fn test_validate_live_reports_an_unparseable_event_log() {
        let (_repo, executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        std::fs::write(data_path(&executor, &VirtualPath::EVENTS), "not jsonl\n").unwrap();

        let error = validate_live(&executor).unwrap_err().to_string();

        assert!(
            error.contains("record-fidelity")
                && error.contains(&VirtualPath::EVENTS.repository_relative())
                && error.contains("line 1"),
            "{error}"
        );
    }

    #[test]
    fn test_overlay_records_dag_and_gate_integrity_judge_planned_bytes() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let id = executor.storage().list_issues().unwrap().remove(0).id;
        let mut issue: Issue = serde_json::from_slice(
            &read_fixture(&executor, &issue_path(&id).repository_relative()).unwrap(),
        )
        .unwrap();
        issue.dependencies = vec![crate::test_utils::issue_id(
            0x424157f2ba6b7bb89e05938a9dbccee5,
        )];
        issue.gates_required = vec!["planned".parse().unwrap()];
        let bad = serde_json::to_vec_pretty(&issue).unwrap();
        let error = format!(
            "{:#}",
            validate_overlaid(
                &executor,
                [
                    (issue_path(&id).repository_relative(), Some(bad)),
                    (
                        VirtualPath::GATES.repository_relative().to_string(),
                        Some(b"[[gates]]\nkey = \"planned\"\ntitle = \"p\"\ndescription = \"\"\nstage = \"postcheck\"\nmode = \"manual\"\n".to_vec()),
                    ),
                ],
            )
            .unwrap_err()
        );
        assert!(
            error.contains("repository-integrity")
                && error.contains(issue.dependencies[0].as_str()),
            "{error}"
        );
    }

    #[test]
    fn test_overlay_namespace_and_hierarchy_judge_planned_config() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let report = validate_materialized_overlaid(
            &executor,
            [put(
                VirtualPath::CONFIG.repository_relative(),
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
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let report = validate_materialized_overlaid(
            &executor,
            [put(
                VirtualPath::RULES.repository_relative(),
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
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let store = executor.storage();
        let layout = executor.require_layout().unwrap();
        store.configure_repository_layout(&layout);
        let mut issue = store.list_issues().unwrap().remove(0);
        issue.dependencies.push(crate::test_utils::issue_id(
            0x96bf59db932e7f49ee5cc6bb5a1f6645,
        ));
        let rules = "[[rules]]\nname = \"task-needs-req\"\nwhen = { type = \"task\" }\n\
                     severity = \"error\"\nenforce = false\n\
                     assert = { require-label = { label = \"req:*\", min = 1 } }\n";
        let live = validate_live(&executor).unwrap();
        assert!(!live.rule_report.has_errors());
        let failure = validate_materialized_overlaid(
            &executor,
            [
                (
                    VirtualPath::RULES.repository_relative().to_string(),
                    Some(rules.as_bytes().to_vec()),
                ),
                (
                    issue_path(&issue.id).repository_relative(),
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
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let config = format!(
            "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n[namespaces.enforces]\ndescription = \"Item link\"\nunique = false\n{}",
            crate::test_utils::shipped_item_kind_tables(&["invariant"])
        );
        let invariants = "[[invariants]]\nid = \"planned\"\nstatement = \"planned bytes win\"\nkind = \"advisory\"\n";
        std::fs::write(
            data_path(&executor, &VirtualPath::CONFIG),
            config.as_bytes(),
        )
        .unwrap();
        std::fs::write(data_path(&executor, &VirtualPath::INVARIANTS), invariants).unwrap();
        let id = &executor.storage().list_issues().unwrap().remove(0).id;
        let mut issue: Issue = serde_json::from_slice(
            &read_fixture(&executor, &issue_path(id).repository_relative()).unwrap(),
        )
        .unwrap();
        issue
            .labels
            .push("enforces:@/invariant/planned".to_string());
        std::fs::write(
            data_path(&executor, &issue_path(id)),
            serde_json::to_vec_pretty(&issue).unwrap(),
        )
        .unwrap();

        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        let live = validate_live(&executor).unwrap();
        assert!(!live.rule_report.has_errors(), "{:?}", live.rule_report);
        let planned = validate_materialized_overlaid(
            &executor,
            [(VirtualPath::INVARIANTS.repository_relative(), None)],
        )
        .unwrap();
        assert!(planned
            .rule_report
            .findings
            .iter()
            .any(|finding| finding.rule == crate::commands::DANGLING_LINK_RULE));
    }

    #[test]
    fn test_validate_live_reports_unqualified_item_link() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let config = format!(
            "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n[namespaces.enforces]\ndescription = \"Item link\"\nunique = false\n{}",
            crate::test_utils::shipped_item_kind_tables(&["invariant"])
        );
        std::fs::write(
            data_path(&executor, &VirtualPath::CONFIG),
            config.as_bytes(),
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        let id = &executor.storage().list_issues().unwrap().remove(0).id;
        let mut issue: Issue = serde_json::from_slice(
            &read_fixture(&executor, &issue_path(id).repository_relative()).unwrap(),
        )
        .unwrap();
        issue.labels.push("enforces:planned".to_string());
        std::fs::write(
            data_path(&executor, &issue_path(id)),
            serde_json::to_vec_pretty(&issue).unwrap(),
        )
        .unwrap();

        let live = validate_live(&executor).unwrap();
        assert!(live.rule_report.findings.iter().any(|finding| {
            finding.rule == crate::commands::DANGLING_LINK_RULE
                && finding.message.contains("'enforces:@/invariant/planned'")
        }));
    }

    #[test]
    fn test_overlay_projection_rejects_stale_planned_target_despite_live_state() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        seed_fixture_issue(
            &mut executor,
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            vec!["type:task".to_string()],
        );
        let config = format!(
            "[type_hierarchy.types]\ntask = 4\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n{}\n\
             [projection.invariants]\nkind = \"invariant\"\ntarget = \"INVARIANTS.md\"\nmode = \"separate-file\"\nstyle = \"full\"\n",
            crate::test_utils::shipped_item_kind_tables(&["invariant"])
        );
        let invariants = "[[invariants]]\nid = \"planned\"\nstatement = \"planned bytes win\"\nkind = \"advisory\"\n";
        let error = validate_overlaid(
            &executor,
            [
                put(VirtualPath::CONFIG.repository_relative(), &config),
                put(VirtualPath::INVARIANTS.repository_relative(), invariants),
                put("INVARIANTS.md", "stale planned projection\n"),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("derived-materialization validation pass"),
            "{error}"
        );
    }
}
