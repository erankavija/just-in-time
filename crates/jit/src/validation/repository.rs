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
    index_items, index_project_sources, is_qualified_reference, load_toml_scope_items,
    unqualified_link_message, ItemAddressIndex, ItemError, ItemKind, ProjectSource, RawScopeItem,
};
use crate::domain::{Issue, ReadinessCorrection};
use crate::graph::DependencyGraph;
use crate::repository_state::{
    captured_issue_ids, parse_issue_records, IssueRecordsError, RepositoryImage, RepositoryIndex,
    VirtualPath,
};
use crate::validation::engine::{Finding, SchemaEngine};
use crate::validation::report::{ReportedFinding, RuleReport};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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

/// A captured image paired with the index and issue records parsed from it.
/// Each loader failure is shared with each report that fails on it; the
/// index's failure precedes the records'.
#[derive(Debug)]
pub struct CapturedValidation<'a> {
    pub(crate) image: Cow<'a, RepositoryImage>,
    pub(crate) index: std::result::Result<RepositoryIndex, Arc<anyhow::Error>>,
    pub(crate) issues: std::result::Result<Vec<Issue>, Arc<anyhow::Error>>,
    /// The JSON value each of `issues` decoded from, in order; empty when they failed.
    pub(crate) values: Vec<serde_json::Value>,
}

impl<'a> CapturedValidation<'a> {
    /// Parse `image`'s index and issue records.
    pub fn from_image(image: &'a RepositoryImage) -> Self {
        match load_issues(image) {
            Ok(records) => {
                let (issues, values) = records.into_iter().unzip();
                Self::new(Cow::Borrowed(image), Ok(issues), values)
            }
            Err(error) => Self::new(Cow::Borrowed(image), Err(error), Vec::new()),
        }
    }

    fn new(
        image: Cow<'a, RepositoryImage>,
        issues: Result<Vec<Issue>>,
        values: Vec<serde_json::Value>,
    ) -> Self {
        Self {
            index: records_pass(load_index(&image)),
            issues: records_pass(issues),
            values,
            image,
        }
    }
}

impl CapturedValidation<'static> {
    /// Parse `image`'s index beside the issue records and values its capture parsed.
    pub(crate) fn with_records(
        image: RepositoryImage,
        issues: Vec<Issue>,
        values: Vec<serde_json::Value>,
    ) -> Self {
        Self::new(Cow::Owned(image), Ok(issues), values)
    }
}

/// Everything a report reads besides its captured validation.
#[derive(Debug)]
pub struct ReportInputs {
    /// The instant `gate-recency` rules judge recorded gate results against.
    pub now: DateTime<Utc>,
    /// Verdicts read from repository history; only whole-repository validation
    /// supplies them.
    pub history: Option<HistoryVerdicts>,
}

/// Verdicts whose evidence lies outside the captured image. Each `Err` holds
/// why the repository fails it.
#[derive(Debug)]
pub struct HistoryVerdicts {
    /// Whether stored records reproduce what the writer would publish.
    pub record_fidelity: std::result::Result<(), String>,
    /// Whether every issue's document references resolve.
    pub document_references: std::result::Result<(), String>,
}

/// The whole-repository validation report.
#[derive(Debug)]
pub struct ValidationReport {
    /// Stages reached in deterministic pipeline order.
    pub passes: Vec<RepositoryValidationPass>,
    /// The first structural failure; every finding collected before it is kept.
    /// A records failure is shared with the [`CapturedValidation`].
    pub structural_failure: Option<Arc<anyhow::Error>>,
    /// Whether the report judged the history verdicts.
    pub has_history: bool,
    /// Declarative and built-in findings, in wire form.
    pub rule_report: RuleReport,
}

impl ValidationReport {
    /// Whether the structural failure judges the repository wrong, where any
    /// other structural failure means validation could not read what it needed.
    pub fn is_judged_invalid(&self) -> bool {
        self.structural_failure
            .as_ref()
            .is_some_and(|error| error.downcast_ref::<RepositoryJudgedInvalid>().is_some())
    }

    /// The structural failure as an owned error, and the findings. The error
    /// keeps its typed causes when the report holds its only reference, as it
    /// does once its [`CapturedValidation`] is dropped; a shared error keeps
    /// only its message.
    pub fn into_parts(self) -> (Option<anyhow::Error>, RuleReport) {
        let failure = self
            .structural_failure
            .map(|error| Arc::try_unwrap(error).unwrap_or_else(|shared| anyhow!("{shared:#}")));
        (failure, self.rule_report)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct RepositoryJudgedInvalid(String);

fn judged_invalid(error: anyhow::Error) -> anyhow::Error {
    RepositoryJudgedInvalid(format!("{error:#}")).into()
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

/// Validate declarations, records, rules, and materializations in the captured
/// image, which is the only evidence source: a live capture validates the
/// working tree, an overlay image
/// ([`apply_overlay`](crate::repository_state::apply_overlay)) a proposed final
/// state.
pub fn report(captured: &CapturedValidation<'_>, inputs: &ReportInputs) -> ValidationReport {
    let image = &*captured.image;
    let mut report = ValidationReport {
        passes: Vec::new(),
        structural_failure: None,
        has_history: inputs.history.is_some(),
        rule_report: RuleReport::default(),
    };
    let declarations = match crate::repository_state::validation_declarations_from_image(image) {
        Ok(declarations) => declarations,
        Err(error) => {
            report.structural_failure = Some(Arc::new(
                anyhow!(error).context("declaration validation pass"),
            ));
            return report;
        }
    };
    let expected = match declarations
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
    {
        Ok(expected) => expected,
        Err(error) => {
            // Whole-repository validation names the pass; a proposed overlay's
            // caller reports the producer's error as it is.
            let error = anyhow!(error);
            report.structural_failure = Some(Arc::new(match inputs.history {
                Some(_) => error.context("derived-materialization validation pass"),
                None => error,
            }));
            return report;
        }
    };
    let read = move |path: &str| image_read(image, path);
    let read: &ReadBytes<'_> = &read;
    let passes = &mut report.passes;
    let config = declarations.config();
    if let Err(error) = config.validate_item_kinds() {
        report.structural_failure = Some(Arc::new(
            anyhow!(error).context("effective-config validation pass"),
        ));
        return report;
    }
    passes.push(RepositoryValidationPass::EffectiveConfig);

    let namespaces = crate::config_manager::namespaces_from_config(config);
    let rules = declarations.rules();
    let rules_loaded = declarations.rules_loaded();
    let findings = &mut report.rule_report.findings;
    if rules_loaded {
        passes.push(RepositoryValidationPass::RulesAndSchemas);
    } else if let Some(error) = declarations.rules_load_error() {
        findings.push(ReportedFinding::new(
            None,
            &Finding {
                rule: crate::validation::report::BuiltInPass::RulesFile
                    .as_str()
                    .to_owned(),
                severity: Severity::Error,
                message: format!("config error: {error:#}"),
            },
        ));
    }

    let mut structural_error = None;
    let gates = declarations.gates();
    passes.push(RepositoryValidationPass::Gates);

    let issues = match captured.index.as_ref().and(captured.issues.as_ref()) {
        Ok(issues) => {
            passes.push(RepositoryValidationPass::Records);
            Some(issues.as_slice())
        }
        Err(error) => {
            structural_error = Some(Arc::clone(error));
            None
        }
    };

    let history = inputs.history.as_ref();
    if let Some(Err(message)) = history.map(|history| &history.record_fidelity) {
        structural_error.get_or_insert(Arc::new(
            anyhow::Error::from(RepositoryJudgedInvalid(message.clone()))
                .context("record-fidelity validation pass"),
        ));
    }

    if let Some(issues) = issues {
        let document_references = history
            .and_then(|history| history.document_references.as_ref().err())
            .map(String::as_str);
        match validate_integrity(issues, gates, document_references)
            .map_err(judged_invalid)
            .context("repository-integrity validation pass")
        {
            Err(error) => {
                structural_error.get_or_insert(Arc::new(error));
            }
            Ok(()) if structural_error.is_none() => {
                passes.push(RepositoryValidationPass::RepositoryIntegrity);
            }
            Ok(()) => {}
        }
    }

    let mut namespace_and_hierarchy_complete = rules_loaded;
    if rules_loaded {
        if let Some(issues) = issues {
            match collect_rule_findings(read, issues, rules, &namespaces, config, inputs.now)
                .context("namespace-and-hierarchy validation pass")
            {
                Ok(rule_findings) => findings.extend(rule_findings),
                Err(error) => {
                    namespace_and_hierarchy_complete = false;
                    structural_error.get_or_insert(Arc::new(error));
                }
            }
        } else {
            namespace_and_hierarchy_complete = false;
        }
    }
    findings.extend(collect_enforcement_drift_findings(&declarations));
    findings.extend(collect_review_placeholder_findings(gates));
    if namespace_and_hierarchy_complete {
        passes.push(RepositoryValidationPass::NamespaceAndHierarchy);
    }

    if let Some(issues) = issues {
        let credit_namespaces = if rules_loaded {
            crate::validation::graph::coverage_credit_namespaces(&rules.rules)
        } else {
            HashSet::new()
        };
        match declarations
            .item_kinds()
            .map_err(anyhow::Error::from)
            .and_then(|kinds| {
                collect_item_link_findings(read, issues, config, kinds, &credit_namespaces)
            })
            .context("item-links validation pass")
        {
            Ok(item_findings) => {
                findings.extend(item_findings);
                passes.push(RepositoryValidationPass::ItemLinks);
            }
            Err(error) => {
                structural_error.get_or_insert(Arc::new(error));
            }
        }
    }

    if let Some(expected) = &expected {
        match validate_materializations(image, expected)
            .context("derived-materialization validation pass")
        {
            Ok(()) => passes.push(RepositoryValidationPass::Projections),
            Err(error) => {
                structural_error.get_or_insert(Arc::new(error));
            }
        }
    }

    report.structural_failure = structural_error;
    report
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

fn records_pass<T>(loaded: Result<T>) -> std::result::Result<T, Arc<anyhow::Error>> {
    loaded.context("records validation pass").map_err(Arc::new)
}

fn load_index(image: &RepositoryImage) -> Result<RepositoryIndex> {
    let index_path = VirtualPath::INDEX.repository_relative();
    RepositoryIndex::parse(
        &image_read(image, &index_path)?
            .ok_or_else(|| anyhow!("required repository file '{index_path}' is missing"))?,
    )
    .context(RepositoryJudgedInvalid(format!("invalid {index_path}")))
}

fn load_issues(image: &RepositoryImage) -> Result<Vec<(Issue, serde_json::Value)>> {
    let ids = captured_issue_ids(image)?;
    parse_issue_records(&ids, |path| image.file_bytes(path)).map_err(|error| match error {
        IssueRecordsError::Malformed { id, error } => {
            anyhow!(error.into_source()).context(RepositoryJudgedInvalid(format!(
                "invalid {}",
                VirtualPath::issue_record(&id).repository_relative()
            )))
        }
        error @ IssueRecordsError::MismatchedId { .. } => {
            RepositoryJudgedInvalid(error.to_string()).into()
        }
        error => error.into(),
    })
}

fn validate_integrity(
    issues: &[Issue],
    gates: &GateRegistry,
    document_references: Option<&str>,
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

    if let Some(reason) = document_references {
        return Err(anyhow!("{reason}"));
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

fn collect_rule_findings(
    read: &ReadBytes<'_>,
    issues: &[Issue],
    rules: &RuleSet,
    namespaces: &crate::domain::LabelNamespaces,
    config: &JitConfig,
    now: DateTime<Utc>,
) -> Result<Vec<ReportedFinding>> {
    let repo_format = config.content_format();
    let engine = SchemaEngine::new();
    let mut reported = Vec::new();
    for issue in issues {
        let evaluation = crate::validation::local::evaluate_local_with_engine(
            issue,
            rules,
            repo_format,
            &engine,
        )?;
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
        now,
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
    declarations: &crate::repository_state::CapturedRepositoryDeclarations,
) -> Vec<ReportedFinding> {
    crate::validation::drift::from_declarations(declarations)
        .into_iter()
        .map(|finding| {
            ReportedFinding::new(
                None,
                &Finding {
                    rule: crate::validation::report::BuiltInPass::EnforcementDrift
                        .as_str()
                        .to_owned(),
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
                rule: crate::validation::report::BuiltInPass::ReviewPlaceholder
                    .as_str()
                    .to_owned(),
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
    kinds: &[ItemKind],
    credit_namespaces: &HashSet<String>,
) -> Result<Vec<ReportedFinding>> {
    if kinds.is_empty() && credit_namespaces.is_empty() {
        return Ok(Vec::new());
    }
    let repo_format = config.content_format();
    let mut addresses = ItemAddressIndex::new(issues);
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
    let local_project = config
        .project
        .as_ref()
        .and_then(|project| project.name.as_ref())
        .map(crate::config::ProjectName::as_str);
    for issue in issues {
        for label in &issue.labels {
            let Some((namespace, value)) = label.split_once(':') else {
                continue;
            };
            if let Some(message) =
                unqualified_link_message(&issue.id, namespace, value, kinds, credit_namespaces)
            {
                findings.push(ReportedFinding::new(
                    Some(issue.id.clone()),
                    &Finding {
                        rule: crate::validation::report::BuiltInPass::DanglingItemLink
                            .as_str()
                            .to_owned(),
                        severity: Severity::Error,
                        message,
                    },
                ));
                continue;
            }
            if !link_namespaces.contains(namespace) || !is_qualified_reference(value) {
                continue;
            }
            let detail = match addresses.contains(value, kinds, local_project) {
                Ok(true) => continue,
                Ok(false) => format!("the qualified id '{value}' resolves to no addressable item"),
                Err(ItemError::NotResolvable { project }) => {
                    format!("addresses project '{project}', not the local project")
                }
                Err(error) => error.to_string(),
            };
            findings.push(ReportedFinding::new(
                Some(issue.id.clone()),
                &Finding {
                    rule: crate::validation::report::BuiltInPass::DanglingItemLink
                        .as_str()
                        .to_owned(),
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

    fn image_report(
        issues: &[Issue],
        item_kinds: &[&str],
        rules: &str,
        gates: &str,
        invariants: &str,
    ) -> ValidationReport {
        let config = format!(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\n\
             description = \"Issue type\"\nunique = true\n{}",
            crate::test_utils::shipped_item_kind_tables(item_kinds)
        );
        let listing = crate::repository_state::DirectoryListing::new(
            issues
                .iter()
                .map(|issue| format!("{}.json", issue.id))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let builder = crate::test_harness::image::ImageBuilder::new()
            .file(VirtualPath::CONFIG, config)
            .file(VirtualPath::RULES, rules)
            .file(VirtualPath::GATES, gates)
            .file(VirtualPath::INVARIANTS, invariants)
            .file(
                VirtualPath::INDEX,
                crate::repository_state::fresh_index_bytes().unwrap(),
            )
            .listing(VirtualPath::ISSUES, listing);
        let builder = issues.iter().fold(builder, |builder, issue| {
            builder.file(
                VirtualPath::issue_record(&issue.id),
                crate::repository_state::serialize_issue(issue).unwrap(),
            )
        });
        overlay_report(&builder.build())
    }

    fn issue_with_labels(title: &str, description: &str, labels: &[&str]) -> Issue {
        let mut issue = crate::test_utils::fixture_issue(title.to_owned(), description.to_owned());
        issue.labels = labels.iter().map(|label| (*label).to_owned()).collect();
        issue
    }

    fn pass_findings(
        report: &ValidationReport,
        pass: crate::validation::report::BuiltInPass,
    ) -> Vec<&ReportedFinding> {
        report
            .rule_report
            .findings
            .iter()
            .filter(|finding| finding.rule == pass.as_str())
            .collect()
    }

    fn item_link_findings(report: &ValidationReport) -> Vec<&ReportedFinding> {
        assert!(report.passes.contains(&RepositoryValidationPass::ItemLinks));
        pass_findings(
            report,
            crate::validation::report::BuiltInPass::DanglingItemLink,
        )
    }

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
    fn test_report_preserves_invalid_hierarchy_error() {
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

        let error = validate_live(&executor)
            .unwrap_err()
            .structural_failure
            .unwrap();

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
    ) -> std::result::Result<ValidationReport, ValidationReport> {
        split(executor.validation().validate_repository_report().unwrap())
    }

    /// Capture and validate the fixture overlaid with proposed final bytes/absence.
    fn validate_overlaid<P: Into<PathBuf>>(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        changes: impl IntoIterator<Item = (P, Option<Vec<u8>>)>,
    ) -> std::result::Result<ValidationReport, ValidationReport> {
        let overrides = crate::commands::overrides_from_repo_changes(
            &executor.require_layout().unwrap(),
            changes
                .into_iter()
                .map(|(path, value)| (path.into(), value)),
        )
        .unwrap();
        let image = executor.capture_validation_image_with(&overrides).unwrap();
        split(overlay_report(&image))
    }

    /// Validate a semantic proposal together with the exact coupled derived-state
    /// writes the canonical materializer would publish for that proposal.
    fn validate_materialized_overlaid<P: Into<PathBuf>>(
        executor: &crate::commands::CommandExecutor<JsonFileStorage>,
        changes: impl IntoIterator<Item = (P, Option<Vec<u8>>)>,
    ) -> std::result::Result<ValidationReport, ValidationReport> {
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
        split(overlay_report(&image))
    }

    /// The report over an overlay image, which supplies no history.
    fn overlay_report(image: &RepositoryImage) -> ValidationReport {
        report(
            &CapturedValidation::from_image(image),
            &ReportInputs {
                now: Utc::now(),
                history: None,
            },
        )
    }

    /// `Err` when `report` carries a structural failure.
    fn split(report: ValidationReport) -> std::result::Result<ValidationReport, ValidationReport> {
        match report.structural_failure {
            Some(_) => Err(report),
            None => Ok(report),
        }
    }

    /// A report's structural failure with its causes.
    fn failure(report: &ValidationReport) -> String {
        format!("{:#}", report.structural_failure.as_ref().unwrap())
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
        assert_eq!(plain.passes, planned.passes);
        assert_eq!(plain.rule_report, planned.rule_report);
        assert_eq!(plain.passes.len(), 8);
    }

    /// Seed one issue holding an attested gate, and name its run.
    fn seed_attested_gate_run(
        executor: &mut crate::commands::CommandExecutor<JsonFileStorage>,
    ) -> (
        crate::domain::identity::IssueId,
        crate::domain::identity::GateRunId,
    ) {
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
        let run_id = executor
            .storage()
            .load_issue(&issue.id)
            .unwrap()
            .gates_status[&key]
            .run_id
            .clone()
            .unwrap();
        (issue.id, run_id)
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
        let (issue_id, run_id) = seed_attested_gate_run(&mut executor);
        let path = VirtualPath::gate_run_record(&issue_id, &run_id);
        let stored = read_fixture(&executor, &path.repository_relative()).unwrap();
        let mut raw: serde_json::Value = serde_json::from_slice(&stored).unwrap();
        raw["retired_key"] = serde_json::Value::Bool(true);
        raw["origin"] = serde_json::json!({ "kind": "not-a-recorded-origin" });
        std::fs::write(
            data_path(&executor, &path),
            serde_json::to_vec(&raw).unwrap(),
        )
        .unwrap();

        let error = failure(&validate_live(&executor).unwrap_err());

        assert!(
            error.contains("record-fidelity") && error.contains(run_id.as_str()),
            "{error}"
        );
    }

    #[test]
    fn test_validate_live_reports_a_verdict_record_that_does_not_reproduce() {
        let (_repo, mut executor) = crate::test_utils::setup_test_repo_with_config(
            "[type_hierarchy.types]\ntask = 4\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
        )
        .unwrap();
        executor
            .validation()
            .validate_with_fix(true, false)
            .unwrap();
        let (issue_id, run_id) = seed_attested_gate_run(&mut executor);
        let path = VirtualPath::gate_verdict_record(
            &"tests".parse().unwrap(),
            &crate::domain::repository_inputs::InputsDigest::try_from("d".repeat(64)).unwrap(),
        );
        let file = data_path(&executor, &path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let record = crate::domain::GateVerdictRecord { issue_id, run_id };
        std::fs::write(&file, serde_json::to_vec(&record).unwrap()).unwrap();

        let error = failure(&validate_live(&executor).unwrap_err());

        assert!(
            error.contains("record-fidelity") && error.contains(&path.repository_relative()),
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
        let (issue_id, run_id) = seed_attested_gate_run(&mut executor);
        std::fs::write(
            data_path(&executor, &VirtualPath::gate_run_record(&issue_id, &run_id)),
            "not json",
        )
        .unwrap();

        validate_overlaid::<&str>(&executor, []).unwrap();
    }

    #[test]
    fn test_image_report_preserves_rules_file_and_enforcement_drift_findings() {
        let report = image_report(
            &[],
            &["invariant"],
            "not toml = [",
            "",
            "[[invariants]]\nid = \"sample-invariant\"\nstatement = \"s\"\n\
             kind = \"enforced\"\nenforced-by = \"@/rule/bad-rule\"\n",
        );
        assert!(report.structural_failure.is_none(), "{report:?}");
        assert!(report.rule_report.findings.iter().any(
            |finding| finding.rule == "rules-file" && finding.message.contains("config error")
        ));
        assert!(report.rule_report.findings.iter().any(|finding| {
            finding.rule == crate::validation::report::BuiltInPass::EnforcementDrift.as_str()
                && finding.message.contains("declared-but-unenforced")
                && finding.message.contains("failed to load")
        }));
        assert!(!report
            .passes
            .contains(&RepositoryValidationPass::Projections));
    }

    #[test]
    fn test_image_report_warns_for_sorted_placeholder_gate_keys() {
        let mut gates = crate::declarations::GateRegistry::default();
        for key in ["z-review", "a-review"] {
            let key: GateKey = key.parse().unwrap();
            gates.gates.insert(
                key.clone(),
                crate::declarations::GateDefinition {
                    version: 1,
                    key: key.clone(),
                    title: key.to_string(),
                    description: String::new(),
                    stage: crate::declarations::GateStage::Postcheck,
                    mode: crate::declarations::GateMode::Manual,
                    checker: Some(crate::declarations::GateChecker::ReviewPlaceholder),
                    inputs: None,
                    priority: 100,
                    reserved: Default::default(),
                },
            );
        }
        let gates =
            String::from_utf8(crate::declarations::serialize_gate_registry(&gates).unwrap())
                .unwrap();
        let report = image_report(&[], &[], "", &gates, "");
        let findings = pass_findings(
            &report,
            crate::validation::report::BuiltInPass::ReviewPlaceholder,
        );

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warn);
        assert!(findings[0].issue_id.is_none());
        assert!(findings[0].message.contains("a-review, z-review"));
    }

    #[test]
    fn test_captured_item_kinds_preserve_effective_configuration_error() {
        let image = crate::test_harness::image::ImageBuilder::new()
            .files(&[
                (
                    ".jit/config.toml",
                    Some("[item_kinds.custom]\nsection = \"Items\""),
                ),
                (".jit/gates.toml", None),
                (".jit/rules.toml", None),
            ])
            .build();
        let declarations =
            crate::repository_state::validation_declarations_from_image(&image).unwrap();
        let expected = declarations.config().validate_item_kinds().unwrap_err();
        let report = overlay_report(&image);
        assert_eq!(
            failure(&report),
            format!("effective-config validation pass: {expected}")
        );
        assert!(report.passes.is_empty());
    }

    #[test]
    fn test_report_names_the_materialization_pass_only_with_history() {
        let config = format!(
            "{}\n[projection.invariants]\nkind = \"invariant\"\ntarget = \"INVARIANTS.md\"\n\
             mode = \"separate-file\"\nstyle = \"full\"\n",
            crate::test_utils::shipped_item_kind_tables(&["invariant"])
        );
        // The projection target lies outside the capture, so derivation fails.
        let image = crate::test_harness::image::ImageBuilder::new()
            .files(&[
                (".jit/config.toml", Some(&config)),
                (".jit/invariants.toml", None),
                (".jit/gates.toml", None),
                (".jit/rules.toml", None),
            ])
            .build();
        let captured = CapturedValidation::from_image(&image);
        let failure_with = |history| {
            failure(&report(
                &captured,
                &ReportInputs {
                    now: Utc::now(),
                    history,
                },
            ))
        };

        let overlay = failure_with(None);
        let whole = failure_with(Some(HistoryVerdicts {
            record_fidelity: Ok(()),
            document_references: Ok(()),
        }));

        assert_eq!(
            whole,
            format!("derived-materialization validation pass: {overlay}")
        );
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
    fn test_report_passes_records_over_an_unparseable_event_log() {
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

        let error = failure(&validate_live(&executor).unwrap_err());

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
        let error = failure(
            &validate_overlaid(
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
        let partial = validate_materialized_overlaid(
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
        assert!(failure(&partial).contains("does not exist"), "{partial:?}");
        assert_eq!(partial.rule_report.error_count(), 1);
        assert_eq!(partial.rule_report.findings[0].rule, "task-needs-req");
    }

    #[test]
    fn test_image_report_resolves_enforces_rule_and_gate_links() {
        let issue = issue_with_labels(
            "node",
            "",
            &[
                "enforces:@/rule/require-success-criteria",
                "enforces:@/gate/cargo-ci",
            ],
        );
        let report = image_report(
            &[issue],
            &["invariant", "rule", "gate"],
            "[[rules]]\nname = \"require-success-criteria\"\n\
             assert = { require-section = { heading = \"Success Criteria\" } }\n",
            "[[gates]]\nversion = 1\nkey = \"cargo-ci\"\ntitle = \"Cargo CI\"\n\
             description = \"CI\"\nstage = \"postcheck\"\nmode = \"manual\"\n",
            "",
        );

        assert!(item_link_findings(&report).is_empty());
    }

    #[test]
    fn test_image_report_finds_unresolvable_enforces_rule_and_gate_links() {
        let issue = issue_with_labels(
            "node",
            "",
            &[
                "enforces:@/rule/no-such-rule",
                "enforces:@/gate/no-such-gate",
            ],
        );
        let report = image_report(
            &[issue],
            &["invariant", "rule", "gate"],
            "[[rules]]\nname = \"require-success-criteria\"\n\
             assert = { require-section = { heading = \"Success Criteria\" } }\n",
            "[[gates]]\nversion = 1\nkey = \"cargo-ci\"\ntitle = \"Cargo CI\"\n\
             description = \"CI\"\nstage = \"postcheck\"\nmode = \"manual\"\n",
            "",
        );
        let findings = item_link_findings(&report);

        assert_eq!(findings.len(), 2);
        assert!(findings
            .iter()
            .any(|finding| finding.message.contains("no-such-rule")));
        assert!(findings
            .iter()
            .any(|finding| finding.message.contains("no-such-gate")));
    }

    #[test]
    fn test_image_report_finds_unresolvable_issue_qualified_id() {
        let target = issue_with_labels(
            "target",
            "## Success Criteria\n\n- [hard] REQ-01: real one\n",
            &[],
        );
        let short = target.short_id();
        let node = issue_with_labels("node", "", &[&format!("satisfies:{short}/BOGUS")]);
        let node_id = node.id.clone();
        let report = image_report(&[target, node], &["invariant", "requirement"], "", "", "");
        let findings = item_link_findings(&report);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].issue_id.as_ref(), Some(&node_id));
        assert_eq!(
            findings[0].rule,
            crate::validation::report::BuiltInPass::DanglingItemLink.as_str()
        );
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(findings[0].message.contains("BOGUS"));
        assert!(findings[0].message.contains("dangling item link"));
    }

    #[test]
    fn test_image_report_resolves_qualified_links_across_item_scopes_and_forms() {
        let target = issue_with_labels(
            "target",
            "## Success Criteria\n\n- [hard] REQ-01: a\n\n\
             ## Decisions\n\n- D-01: a\n\n## Risks\n\n- RISK-01: a\n",
            &[],
        );
        let short = target.short_id();
        let node = issue_with_labels(
            "node",
            "",
            &[
                &format!("satisfies:{short}/REQ-01"),
                &format!("per:{short}/D-01"),
                &format!("mitigates:{short}/RISK-01"),
                "enforces:@/invariant/sample-invariant",
                &format!("satisfies:@/issue/{short}/requirement/REQ-01"),
            ],
        );
        let report = image_report(
            &[target, node],
            &["invariant", "requirement", "decision", "risk"],
            "",
            "",
            "[[invariants]]\nid = \"sample-invariant\"\n\
             statement = \"Every dependency edge stays acyclic.\"\nkind = \"enforced\"\n",
        );

        let findings = item_link_findings(&report);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn test_image_report_finds_named_project_link_without_panicking() {
        let issue = issue_with_labels("node", "", &["enforces:@acme/invariant/sample-invariant"]);
        let report = image_report(
            &[issue],
            &["invariant"],
            "",
            "",
            "[[invariants]]\nid = \"sample-invariant\"\n\
             statement = \"Every dependency edge stays acyclic.\"\nkind = \"enforced\"\n",
        );
        let findings = item_link_findings(&report);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("project 'acme'"));
        assert!(findings[0].message.contains("invariant/sample-invariant"));
    }

    #[test]
    fn test_image_report_finds_unresolvable_explicit_invariant_address() {
        let issue = issue_with_labels("node", "", &["enforces:@/invariant/missing-invariant"]);
        let report = image_report(
            &[issue],
            &["invariant"],
            "",
            "",
            "[[invariants]]\nid = \"sample-invariant\"\n\
             statement = \"Every dependency edge stays acyclic.\"\nkind = \"enforced\"\n",
        );
        let findings = item_link_findings(&report);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("missing-invariant"));
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
        assert!(planned.rule_report.findings.iter().any(|finding| {
            finding.rule == crate::validation::report::BuiltInPass::DanglingItemLink.as_str()
        }));
    }

    #[test]
    fn test_image_report_reports_unqualified_links_and_ignores_other_namespaces() {
        let issue = issue_with_labels(
            "Root",
            "## Success Criteria\n\n- [hard] REQ-01: planned bytes\n",
            &[
                "type:task",
                "epic:foo",
                "enforces:planned",
                "satisfies:REQ-01",
            ],
        );
        let report = image_report(
            &[issue],
            &["invariant", "requirement"],
            "",
            "",
            "[[invariants]]\nid = \"planned\"\nstatement = \"planned bytes win\"\n\
             kind = \"advisory\"\n",
        );
        let findings = item_link_findings(&report);

        assert_eq!(findings.len(), 2);
        assert!(findings
            .iter()
            .any(|finding| finding.message.contains("'enforces:@/invariant/planned'")));
        assert!(findings
            .iter()
            .any(|finding| finding.message.contains("'satisfies:REQ-01'")));
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
        let error = failure(
            &validate_overlaid(
                &executor,
                [
                    put(VirtualPath::CONFIG.repository_relative(), &config),
                    put(VirtualPath::INVARIANTS.repository_relative(), invariants),
                    put("INVARIANTS.md", "stale planned projection\n"),
                ],
            )
            .unwrap_err(),
        );
        assert!(
            error.contains("derived-materialization validation pass"),
            "{error}"
        );
    }
}
