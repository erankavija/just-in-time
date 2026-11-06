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
use crate::domain::{Issue, ReadinessCorrection, State};
use crate::graph::DependencyGraph;
use crate::repository_state::{
    captured_issue_ids, parse_issue_records, IssueRecordsError, RepositoryImage, RepositoryIndex,
    VirtualPath,
};
use crate::validation::engine::SchemaEngine;
use crate::validation::report::{BuiltInPass, ReportFinding, RuleReport, Selectable};
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
    pub(crate) issues: std::result::Result<Arc<[Issue]>, Arc<anyhow::Error>>,
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
            issues: records_pass(issues).map(Arc::from),
            values,
            image,
        }
    }
}

impl CapturedValidation<'static> {
    /// Parse `image`'s index beside the issue records and values its capture parsed.
    pub(crate) fn with_records(
        image: RepositoryImage,
        issues: Result<Vec<Issue>>,
        values: Vec<serde_json::Value>,
    ) -> Self {
        Self::new(Cow::Owned(image), issues, values)
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
    /// Declarative and built-in findings with their provenance.
    pub findings: Vec<ReportFinding>,
    /// What selections read, or the failure that left them nothing to read.
    pub(super) selectable: std::result::Result<Box<Selectable>, Arc<anyhow::Error>>,
}

impl ValidationReport {
    /// Whether the structural failure judges the repository wrong, where any
    /// other structural failure means validation could not read what it needed.
    pub fn is_judged_invalid(&self) -> bool {
        self.structural_failure
            .as_ref()
            .is_some_and(|error| error.downcast_ref::<RepositoryJudgedInvalid>().is_some())
    }

    /// Every finding in wire form.
    pub fn rule_report(&self) -> RuleReport {
        RuleReport {
            findings: self
                .findings
                .iter()
                .map(|finding| finding.reported.clone())
                .collect(),
        }
    }

    /// The structural failure as an owned error, and the findings. The error
    /// keeps its typed causes when the report holds its only reference, as it
    /// does once its [`CapturedValidation`] is dropped; a shared error keeps
    /// only its message.
    pub fn into_parts(self) -> (Option<anyhow::Error>, RuleReport) {
        let rule_report = self.rule_report();
        let Self {
            structural_failure,
            selectable,
            ..
        } = self;
        drop(selectable);
        let failure = structural_failure
            .map(|error| Arc::try_unwrap(error).unwrap_or_else(|shared| anyhow!("{shared:#}")));
        (failure, rule_report)
    }
}

/// The first template whose node types the declared hierarchy lacks.
///
/// A node type is a declaration a template names whatever it is applied to;
/// its other requirements are prerequisites of applying it, judged by
/// `jit apply` over the same computation.
fn undeclared_template_node_types(
    declarations: &crate::repository_state::CapturedRepositoryDeclarations,
) -> Option<crate::templates::TemplatePrerequisitesUnmetError> {
    let config = declarations.config();
    config.templates.templates.iter().find_map(|template| {
        let types = template
            .unmet_requirements(config, declarations.gates(), &HashMap::new())
            .into_iter()
            .filter(|requirement| {
                matches!(requirement, crate::templates::TemplateRequirement::Type(_))
            })
            .collect::<Vec<_>>();
        (!types.is_empty())
            .then(|| crate::templates::TemplatePrerequisitesUnmetError::new(&template.name, types))
    })
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
    let failed = |error: anyhow::Error| {
        let error = Arc::new(error);
        ValidationReport {
            passes: Vec::new(),
            structural_failure: Some(Arc::clone(&error)),
            has_history: inputs.history.is_some(),
            findings: Vec::new(),
            selectable: Err(error),
        }
    };
    let declarations = match crate::repository_state::validation_declarations_from_image(image) {
        Ok(declarations) => declarations,
        Err(error) => return failed(anyhow!(error).context("declaration validation pass")),
    };
    if let Some(error) = undeclared_template_node_types(&declarations) {
        return failed(anyhow!(error).context("declaration validation pass"));
    }
    let expected = match declarations
        .rules_loaded()
        .then(|| {
            crate::repository_state::derive_materialization(
                image,
                &crate::repository_state::MutationContext::preview(),
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
            return failed(match inputs.history {
                Some(_) => error.context("derived-materialization validation pass"),
                None => error,
            });
        }
    };
    let read = move |path: &str| image_read(image, path);
    let read: &ReadBytes<'_> = &read;
    let config = declarations.config();
    let mut passes = vec![RepositoryValidationPass::EffectiveConfig];

    let namespaces = crate::config_manager::namespaces_from_config(config);
    let rules = declarations.rules();
    let rules_loaded = declarations.rules_loaded();
    let mut findings = Vec::new();
    let rules_file_message = declarations
        .rules_load_error()
        .map(|error| format!("config error: {error:#}"));
    match &rules_file_message {
        Some(message) => findings.push(ReportFinding::built_in(
            BuiltInPass::RulesFile,
            None,
            Severity::Error,
            message.clone(),
        )),
        None => passes.push(RepositoryValidationPass::RulesAndSchemas),
    }

    let mut structural_error = None;
    let gates = declarations.gates();
    passes.push(RepositoryValidationPass::Gates);

    let issues = match captured.index.as_ref().and(captured.issues.as_ref()) {
        Ok(issues) => {
            passes.push(RepositoryValidationPass::Records);
            Some(&issues[..])
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
    // A rule pass that failed leaves every selection's findings incomplete.
    let mut rule_pass_error = None;
    if rules_loaded {
        if let Some(issues) = issues {
            match collect_rule_findings(read, issues, rules, &namespaces, config, inputs.now)
                .context("namespace-and-hierarchy validation pass")
            {
                Ok(rule_findings) => findings.extend(rule_findings),
                Err(error) => {
                    namespace_and_hierarchy_complete = false;
                    let error = Arc::new(error);
                    structural_error.get_or_insert(Arc::clone(&error));
                    rule_pass_error = Some(error);
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

    let selectable = match (
        captured.index.as_ref().and(captured.issues.as_ref()),
        rule_pass_error,
    ) {
        (Ok(records), None) => Ok(Box::new(Selectable {
            records: Arc::clone(records),
            rules: rules_file_message.map_or_else(|| Ok(declarations.rules), Err),
            templates: declarations.config.templates,
        })),
        (Err(error), _) => Err(Arc::clone(error)),
        (Ok(_), Some(error)) => Err(error),
    };
    ValidationReport {
        passes,
        structural_failure: structural_error,
        has_history: inputs.history.is_some(),
        findings,
        selectable,
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
            if !issue.state.is_terminal()
                && issue.state != State::Archived
                && !gates.gates.contains_key(gate)
            {
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
) -> Result<Vec<ReportFinding>> {
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
                .map(|finding| ReportFinding::local(&issue.id, finding)),
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
    reported.extend(graph_findings.iter().map(|finding| {
        let repo_wide = graph_rules.iter().any(|rule| {
            rule.name == finding.finding.rule && rule.assert.is_repo_wide_at_transition()
        });
        ReportFinding::graph(finding, repo_wide)
    }));
    Ok(reported)
}

fn collect_enforcement_drift_findings(
    declarations: &crate::repository_state::CapturedRepositoryDeclarations,
) -> Vec<ReportFinding> {
    crate::validation::drift::from_declarations(declarations)
        .into_iter()
        .map(|finding| {
            ReportFinding::built_in(
                BuiltInPass::EnforcementDrift,
                None,
                Severity::Error,
                finding.message(),
            )
        })
        .collect()
}

fn collect_review_placeholder_findings(gates: &GateRegistry) -> Vec<ReportFinding> {
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
        vec![ReportFinding::built_in(
            BuiltInPass::ReviewPlaceholder,
            None,
            Severity::Warn,
            format!(
                "WARNING: passing external-review placeholder still configured for gate(s): {}. Replace each placeholder with a real review checker before relying on these gates.",
                keys.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
            ),
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
) -> Result<Vec<ReportFinding>> {
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
                findings.push(ReportFinding::built_in(
                    BuiltInPass::DanglingItemLink,
                    Some(issue.id.clone()),
                    Severity::Error,
                    message,
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
            findings.push(ReportFinding::built_in(
                BuiltInPass::DanglingItemLink,
                Some(issue.id.clone()),
                Severity::Error,
                format!(
                    "issue {} has a dangling item link '{label}': {detail}",
                    issue.short_id()
                ),
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
        let image = builder.build();
        let declarations =
            crate::repository_state::validation_declarations_from_image(&image).unwrap();
        let image = if declarations.rules_loaded() {
            let serialized =
                crate::repository_state::serialize_ruleset(declarations.rules()).unwrap();
            crate::repository_state::apply_overlay(
                &image,
                std::iter::once((VirtualPath::RULES, Some(serialized.rules_toml.into_bytes())))
                    .chain(serialized.schema_files.into_iter().map(|schema| {
                        (
                            VirtualPath::schema(&schema.name).unwrap(),
                            Some(schema.content.into_bytes()),
                        )
                    })),
            )
            .unwrap()
        } else {
            image
        };
        overlay_report(&image)
    }

    fn issue_with_labels(title: &str, description: &str, labels: &[&str]) -> Issue {
        let mut issue = crate::test_utils::fixture_issue(title.to_owned(), description.to_owned());
        issue.labels = labels.iter().map(|label| (*label).to_owned()).collect();
        issue
    }

    fn pass_findings(
        report: &ValidationReport,
        pass: crate::validation::report::BuiltInPass,
    ) -> Vec<&crate::validation::ReportedFinding> {
        report
            .findings
            .iter()
            .map(|finding| &finding.reported)
            .filter(|finding| finding.rule == pass.as_str())
            .collect()
    }

    fn item_link_findings(report: &ValidationReport) -> Vec<&crate::validation::ReportedFinding> {
        assert!(report.passes.contains(&RepositoryValidationPass::ItemLinks));
        pass_findings(
            report,
            crate::validation::report::BuiltInPass::DanglingItemLink,
        )
    }

    #[test]
    fn test_validate_integrity_allows_retired_gates_only_on_closed_issues() {
        let mut issue = issue_with_labels("Historical", "", &[]);
        issue.gates_required.push("retired".parse().unwrap());
        let gates = GateRegistry::default();
        for state in [State::Done, State::Rejected, State::Archived] {
            issue.state = state;
            assert!(validate_integrity(&[issue.clone()], &gates, None).is_ok());
        }
        for state in [
            State::Backlog,
            State::Ready,
            State::InProgress,
            State::Gated,
        ] {
            issue.state = state;
            assert!(validate_integrity(&[issue.clone()], &gates, None).is_err());
        }
    }

    #[test]
    fn test_report_preserves_invalid_hierarchy_error() {
        let image = crate::test_harness::image::ImageBuilder::new()
            .file(
                VirtualPath::CONFIG,
                crate::test_taxonomy::invalid_hierarchy_config_fragment(),
            )
            .entry(
                VirtualPath::GATES,
                crate::repository_state::RepositoryEntry::Absent,
            )
            .entry(
                VirtualPath::RULES,
                crate::repository_state::RepositoryEntry::Absent,
            )
            .file(
                VirtualPath::INDEX,
                crate::repository_state::fresh_index_bytes().unwrap(),
            )
            .listing(
                VirtualPath::ISSUES,
                crate::repository_state::DirectoryListing::new([]).unwrap(),
            )
            .build();
        let error = overlay_report(&image).structural_failure.unwrap();

        assert_eq!(
            error.downcast_ref::<crate::domain::type_taxonomy::ConfigError>(),
            Some(&crate::domain::type_taxonomy::ConfigError::InvalidLevel(0))
        );
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

    /// A report's structural failure with its causes.
    fn failure(report: &ValidationReport) -> String {
        format!("{:#}", report.structural_failure.as_ref().unwrap())
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
        assert!(report.rule_report().findings.iter().any(
            |finding| finding.rule == "rules-file" && finding.message.contains("config error")
        ));
        assert!(report.rule_report().findings.iter().any(|finding| {
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
    fn test_report_passes_records_over_an_unparseable_event_log() {
        let image = crate::test_harness::image::ImageBuilder::new()
            .file(VirtualPath::CONFIG, "")
            .entry(
                VirtualPath::RULES,
                crate::repository_state::RepositoryEntry::Absent,
            )
            .file(VirtualPath::GATES, "")
            .file(
                VirtualPath::INDEX,
                crate::repository_state::fresh_index_bytes().unwrap(),
            )
            .listing(
                VirtualPath::ISSUES,
                crate::repository_state::DirectoryListing::new([]).unwrap(),
            )
            .file(VirtualPath::EVENTS, "not jsonl\n")
            .build();
        let report = overlay_report(&image);

        assert!(report.structural_failure.is_none(), "{report:?}");
        assert!(report.passes.contains(&RepositoryValidationPass::Records));
    }

    #[test]
    fn test_repository_view_keeps_non_enforced_rule_error_reportable() {
        let mut issue = issue_with_labels("Root", "", &["type:task"]);
        issue.state = crate::domain::State::Ready;
        let report = image_report(
            &[issue],
            &[],
            "[[rules]]\nname = \"task-needs-req\"\nwhen = { type = \"task\" }\n\
             severity = \"error\"\nenforce = false\n\
             assert = { require-label = { label = \"req:*\", min = 1 } }\n",
            "",
            "",
        );
        assert!(report.structural_failure.is_none(), "{report:?}");
        assert_eq!(report.rule_report().error_count(), 1);
        assert_eq!(report.rule_report().findings[0].rule, "task-needs-req");
    }

    #[test]
    fn test_repository_failure_retains_partial_rule_report() {
        let mut issue = issue_with_labels("Root", "", &["type:task"]);
        issue.state = crate::domain::State::Ready;
        let live = image_report(&[issue.clone()], &[], "", "", "");
        assert!(live.structural_failure.is_none(), "{live:?}");
        assert!(!live.rule_report().has_errors());
        issue.dependencies.push(crate::test_utils::issue_id(
            0x96bf59db932e7f49ee5cc6bb5a1f6645,
        ));
        let rules = "[[rules]]\nname = \"task-needs-req\"\nwhen = { type = \"task\" }\n\
                     severity = \"error\"\nenforce = false\n\
                     assert = { require-label = { label = \"req:*\", min = 1 } }\n";
        let partial = image_report(&[issue], &[], rules, "", "");
        assert!(failure(&partial).contains("does not exist"), "{partial:?}");
        assert_eq!(partial.rule_report().error_count(), 1);
        assert_eq!(partial.rule_report().findings[0].rule, "task-needs-req");
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
}
