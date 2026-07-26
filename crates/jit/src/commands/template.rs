//! Graph-template apply engine.
//!
//! Expansion is pure. Publication derives issues, gates, and events from one
//! captured repository image, then submits one typed materialization plan so the
//! complete scaffold is published atomically. Force-refresh updates prose only;
//! replaying transforms over a live scaffold would corrupt its dependency spine.
//!
//! Types, gates, document locations, labels, roles, and anchors come from the
//! repository template rather than hardcoded workflow vocabulary
//! (`@/inv/domain-agnostic`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::template_expand::{
    expand_template, node_description, validate_delta_acyclic, DeltaEndpoint, InterpolationContext,
    PlannedNode, TemplateDelta,
};
use super::*;
use crate::repository_state::{
    finalize, finalize_gate_registry_edit, render_capture_closure, CaptureBudget, CaptureSpec,
    MutationContext, MutationIntent, RepositoryEntry, RepositoryImage, VirtualPath,
};
use crate::storage::{
    AmbiguousIdError, InvalidIdPrefixError, IssueNotFoundError, MIN_ID_PREFIX_LENGTH,
};
use crate::templates::{GraphTemplate, RoleBindings};
use serde::Serialize;

/// Actor recorded on the events the apply engine appends directly.
const APPLY_ACTOR: &str = "agent:apply";

const TEMPLATE_CAPTURE_BUDGET: CaptureBudget = CaptureBudget {
    max_paths: 1 << 16,
    max_listings: 2,
    max_bytes: 512 * 1024 * 1024,
    max_depth: 16,
};

/// How a template node/anchor gate NAME resolves: a registered gate PRESET
/// bundle, or a single gate KEY declared in the gate registry (`.jit/gates.toml`).
///
/// Resolving anchors/nodes against BOTH lets a config-declared gate (e.g.
/// `repo-validate`) be referenced from a template without being a built-in
/// preset, keeping engine code free of any gate/container literal.
enum TemplateGateResolution {
    /// A registered gate preset and its captured definition.
    Preset(crate::gate_presets::GatePresetDefinition),
    /// A single gate key in the registry; attach that one gate via `add_gates`.
    RegistryKey,
}

enum TemplateRequest<'a> {
    Named(&'a str),
    Explicit(&'a GraphTemplate),
}

impl TemplateRequest<'_> {
    fn resolve(&self, config: &crate::config::JitConfig) -> Result<GraphTemplate> {
        match self {
            Self::Named(name) => config.templates.get(name).cloned().ok_or_else(|| {
                anyhow!("no template '{name}' in .jit/templates.toml; declare it or check the name")
            }),
            Self::Explicit(template) => Ok((*template).clone()),
        }
    }
}

/// Outcome of applying a graph template to a container.
///
/// Names the template applied and the anchor bindings used, maps each created
/// node's template ROLE to the id of the issue created (or refreshed) for it,
/// and carries the PRE-APPLY snapshot of each bound anchor's dependencies. The
/// snapshot is what the `move-upstream-to-role` transform moves onto the node of
/// the role its own `role` field names; capturing it before any mutation is what
/// lets the transform move exactly
/// the container's ORIGINAL upstream deps (and never the freshly-wired scaffold
/// edges). It is also surfaced for callers/tests that inspect the pre-apply
/// shape.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TemplateApplyResult {
    /// The applied template's name (e.g. `"plan"`).
    pub template: String,
    /// Anchor name → bound issue id (full id), in anchor-name order.
    pub anchor_bindings: BTreeMap<String, String>,
    /// Template node role → the created (or `--force`-refreshed) issue id.
    pub created_node_ids_by_role: BTreeMap<String, String>,
    /// Anchor name → that anchor's `dependencies` as snapshotted BEFORE any
    /// mutation. Consumed by the `move-upstream-to-role` transform and surfaced
    /// for callers inspecting the pre-apply shape.
    pub anchor_dependency_snapshots: BTreeMap<String, Vec<String>>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The repository's [`RoleBindings`]: the names its templates give the
    /// planning and breakdown nodes (`.jit/templates.toml`'s `[roles]` table).
    ///
    /// The single lookup every consumer (apply, refresh, breakdown, validation)
    /// asks for the bracket roles through, so no command reads the config keys
    /// itself.
    pub(crate) fn template_roles(&self) -> Result<&RoleBindings> {
        Ok(&self.cached_config()?.templates.roles)
    }

    /// The repository's name for the anchor `jit apply <template> <container>`
    /// binds to its positional `<container>` argument
    /// (`.jit/templates.toml`'s `[anchors] container`, defaulting to
    /// [`DEFAULT_CONTAINER_ANCHOR`](crate::templates::DEFAULT_CONTAINER_ANCHOR)).
    pub fn container_anchor(&self) -> Result<&str> {
        Ok(self.cached_config()?.templates.anchors.container_anchor())
    }

    /// Apply a graph template named `template_name` to `container_id`
    /// (`jit apply <template> <container>`).
    ///
    /// Reads the template and default container-anchor name from the captured
    /// `.jit/templates.toml`. `anchor_bindings` supplies explicit bindings, which
    /// override the captured positional-container default.
    pub fn apply_template(
        &self,
        template_name: &str,
        container_id: &str,
        anchor_bindings: &BTreeMap<String, String>,
        force: bool,
    ) -> Result<(TemplateApplyResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.apply_template_request(
            TemplateRequest::Named(template_name),
            container_id,
            anchor_bindings,
            force,
        )
    }

    /// Apply an explicit [`GraphTemplate`] — the registry-independent core of
    /// [`apply_template`](Self::apply_template).
    ///
    /// This request value is explicit, but all repository declarations and issue
    /// records still come from one bounded captured image. Fresh application and
    /// `--force` refresh both publish one typed transaction; a conflict recaptures
    /// and re-derives while preserving operation-scoped identifiers and time.
    pub fn apply_template_with(
        &self,
        template: &GraphTemplate,
        container_id: &str,
        anchor_bindings: &BTreeMap<String, String>,
        force: bool,
    ) -> Result<(TemplateApplyResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.apply_template_request(
            TemplateRequest::Explicit(template),
            container_id,
            anchor_bindings,
            force,
        )
    }

    fn apply_template_request(
        &self,
        request: TemplateRequest<'_>,
        container_id: &str,
        anchor_bindings: &BTreeMap<String, String>,
        force: bool,
    ) -> Result<(TemplateApplyResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let layout = self.require_layout()?;
        let context = MutationContext::production();
        with_mutation_attempts("template apply", || {
            let (lease_targets, lease_mode) = {
                let mut session = self.storage.open_mutation_session(layout.clone())?;
                let Some(image) =
                    capture_template_image(&mut *session, &context, &request, container_id)?
                else {
                    return Ok(AttemptOutcome::Retry);
                };
                let captured = derive_captured_template_apply(
                    &image,
                    &request,
                    container_id,
                    anchor_bindings,
                    force,
                    &context,
                )?;
                (
                    existing_template_lease_targets(
                        &captured.issues,
                        &captured.derived.lease_targets,
                    ),
                    self.config_manager
                        .enforcement_mode_from_config(&captured.config)?,
                )
            };
            let warnings = template_lease_warnings(lease_mode, &lease_targets)?;

            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(image) =
                capture_template_image(&mut *session, &context, &request, container_id)?
            else {
                return Ok(AttemptOutcome::Retry);
            };
            let mut captured = derive_captured_template_apply(
                &image,
                &request,
                container_id,
                anchor_bindings,
                force,
                &context,
            )?;
            if self
                .config_manager
                .enforcement_mode_from_config(&captured.config)?
                != lease_mode
                || existing_template_lease_targets(
                    &captured.issues,
                    &captured.derived.lease_targets,
                ) != lease_targets
            {
                return Ok(AttemptOutcome::Retry);
            }
            if captured.derived.intents.is_empty() {
                return Ok(AttemptOutcome::Done((captured.derived.result, warnings)));
            }
            captured.declarations.gates = captured.derived.registry;
            let edits_gate_registry = captured
                .derived
                .intents
                .iter()
                .any(|intent| matches!(intent, MutationIntent::EditGateRegistry { .. }));
            let plan = if edits_gate_registry && image.file_bytes(&VirtualPath::CONFIG)?.is_some() {
                finalize_gate_registry_edit(
                    &layout,
                    &image,
                    &context,
                    &captured.derived.intents,
                    captured.declarations.borrowed(),
                )?
            } else {
                // An absent config is a supported default repository, so there is
                // no authored declaration document for repository validation to
                // parse. The same typed finalizer still owns every issue, gate
                // declaration, and event byte.
                finalize(&layout, &image, &context, &captured.derived.intents)?
            };
            classify_apply(session.apply(&plan), (captured.derived.result, warnings))
        })
    }
}

fn captured_anchor_bindings(
    request: &TemplateRequest<'_>,
    config: &crate::config::JitConfig,
    container_id: &str,
    explicit: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut bindings = explicit.clone();
    if matches!(request, TemplateRequest::Named(_)) {
        bindings
            .entry(config.templates.anchors.container_anchor().to_string())
            .or_insert_with(|| container_id.to_string());
    }
    bindings
}

fn existing_template_lease_targets(issues: &[Issue], targets: &[String]) -> Vec<String> {
    let existing = issues
        .iter()
        .map(|issue| issue.id.as_str())
        .collect::<HashSet<_>>();
    targets
        .iter()
        .filter(|id| existing.contains(id.as_str()))
        .cloned()
        .collect()
}

fn template_lease_warnings(
    mode: crate::config::EnforcementMode,
    targets: &[String],
) -> Result<Vec<String>> {
    use crate::agent_config::resolve_agent_id;
    use crate::config::EnforcementMode;
    use crate::storage::claim_coordinator::ClaimsIndex;
    use crate::storage::worktree_paths::WorktreePaths;

    if mode == EnforcementMode::Off {
        return Ok(Vec::new());
    }
    let claims = WorktreePaths::detect()
        .ok()
        .map(|paths| ClaimsIndex::load(&paths))
        .transpose()?;
    let agent = resolve_agent_id(None).ok();
    let now = chrono::Utc::now();
    let mut warnings = Vec::new();
    for id in targets {
        let active = claims.as_ref().is_some_and(|claims| {
            claims.leases.iter().any(|lease| {
                lease.issue_id == *id
                    && lease.expires_at.is_none_or(|expires| expires > now)
                    && !claims.is_stale(lease)
                    && agent
                        .as_ref()
                        .is_none_or(|agent_id| lease.agent_id == *agent_id)
            })
        });
        if active {
            continue;
        }
        let message =
            format!("No active lease for issue {id}.\nAcquire lease with: jit claim acquire {id}");
        match mode {
            EnforcementMode::Warn => warnings.push(message),
            EnforcementMode::Strict => return Err(anyhow!(message)),
            EnforcementMode::Off => unreachable!(),
        }
    }
    Ok(warnings)
}

struct DerivedTemplateApply {
    result: TemplateApplyResult,
    registry: crate::declarations::GateRegistry,
    intents: Vec<MutationIntent>,
    lease_targets: Vec<String>,
}

struct CapturedTemplateApply {
    config: crate::config::JitConfig,
    declarations: crate::repository_state::CapturedRepositoryDeclarations,
    issues: Vec<Issue>,
    derived: DerivedTemplateApply,
}

fn derive_captured_template_apply(
    image: &RepositoryImage,
    request: &TemplateRequest<'_>,
    container_id: &str,
    anchor_bindings: &BTreeMap<String, String>,
    force: bool,
    context: &MutationContext,
) -> Result<CapturedTemplateApply> {
    let config = template_config_from_image(image)?;
    let template = request.resolve(&config)?;
    let bindings = captured_anchor_bindings(request, &config, container_id, anchor_bindings);
    let issues = parse_template_issues(image)?;
    let presets = parse_template_presets(image)?;
    let registry = parse_template_gate_registry(image)?;
    let declarations = template_declarations_from_image(image, &config)?;
    let derived = derive_template_apply(
        &template,
        container_id,
        &bindings,
        force,
        &config.templates.roles,
        &issues,
        registry,
        &presets,
        &config,
        &declarations.rules,
        context,
    )?;
    Ok(CapturedTemplateApply {
        config,
        declarations,
        issues,
        derived,
    })
}

struct FreshTemplateDerivation {
    created: BTreeMap<String, String>,
    candidate: BTreeMap<String, Issue>,
    created_order: Vec<String>,
    lease_targets: Vec<String>,
}

fn template_fixed_paths() -> Vec<VirtualPath> {
    vec![
        VirtualPath::CONFIG,
        VirtualPath::INDEX,
        VirtualPath::EVENTS,
        VirtualPath::GATES,
        VirtualPath::TEMPLATES,
        VirtualPath::RULES,
        VirtualPath::INVARIANTS,
        VirtualPath::ISSUES,
        VirtualPath::GATE_PRESETS,
    ]
}

fn capture_template_image(
    session: &mut dyn crate::storage::RepositoryMutationSession,
    context: &MutationContext,
    request: &TemplateRequest<'_>,
    container_id: &str,
) -> Result<Option<RepositoryImage>> {
    let presets_dir = VirtualPath::GATE_PRESETS;
    let issues_dir = VirtualPath::ISSUES;
    let mut first_spec = CaptureSpec::phase_one(template_fixed_paths(), TEMPLATE_CAPTURE_BUDGET)?;
    first_spec.discover_listing(presets_dir.clone())?;
    let Some(first) = capture_or_retry(session.capture(first_spec))? else {
        return Ok(None);
    };
    let discovered_index = parse_template_index(&first)?;
    let discovered_presets = listed_json_paths(&first, &presets_dir)?;
    let closure = template_declaration_closure(&first)?;

    let mut spec = CaptureSpec::phase_one(template_fixed_paths(), TEMPLATE_CAPTURE_BUDGET)?;
    let mut discovered_paths = discovered_index
        .all_ids
        .iter()
        .map(|id| VirtualPath::data(format!("issues/{id}.json")).map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;
    discovered_paths.extend(discovered_presets.iter().cloned());
    discovered_paths.extend(closure.iter().cloned());
    spec.discover_paths(discovered_paths)?;
    spec.discover_listing(issues_dir)?;
    spec.discover_listing(presets_dir.clone())?;
    let Some(mut image) = capture_or_retry(session.capture(spec.clone()))? else {
        return Ok(None);
    };
    if !template_static_capture_matches(
        &image,
        &discovered_index,
        &presets_dir,
        &discovered_presets,
        &closure,
    )? {
        return Ok(None);
    }

    let derived = template_operation_capture_paths(&image, context, request, container_id)?;
    if derived
        .iter()
        .all(|path| image.capture_spec().contains_path(path))
    {
        return Ok(Some(image));
    }
    spec.discover_paths(derived.iter().cloned())?;
    let Some(next_image) = capture_or_retry(session.capture(spec))? else {
        return Ok(None);
    };
    image = next_image;
    if !template_static_capture_matches(
        &image,
        &discovered_index,
        &presets_dir,
        &discovered_presets,
        &closure,
    )? || template_operation_capture_paths(&image, context, request, container_id)? != derived
    {
        return Ok(None);
    }
    Ok(Some(image))
}

fn template_static_capture_matches(
    image: &RepositoryImage,
    index: &crate::repository_state::RepositoryIndex,
    presets_dir: &VirtualPath,
    presets: &[VirtualPath],
    closure: &[VirtualPath],
) -> Result<bool> {
    let captured = parse_template_index(image)?;
    Ok(captured.all_ids == index.all_ids
        && captured.deleted_ids == index.deleted_ids
        && listed_json_paths(image, presets_dir)? == presets
        && template_declaration_closure(image)? == closure)
}

fn template_declaration_closure(image: &RepositoryImage) -> Result<Vec<VirtualPath>> {
    let config = template_config_from_image(image)?;
    let rules = image
        .file_bytes(&VirtualPath::RULES)?
        .map(std::str::from_utf8)
        .transpose()?;
    Ok(render_capture_closure(image.layout(), &config, &[], rules)?)
}

fn template_operation_capture_paths(
    image: &RepositoryImage,
    context: &MutationContext,
    request: &TemplateRequest<'_>,
    container_id: &str,
) -> Result<Vec<VirtualPath>> {
    let config = template_config_from_image(image)?;
    let template = request.resolve(&config)?;
    let issues = parse_template_issues(image)?;
    let full_container_id = resolve_template_issue_id(&issues, container_id)?;
    let container = template_issue(&issues, &full_container_id)?;
    let mut paths = template
        .nodes
        .iter()
        .filter(|node| node.role == config.templates.roles.planning_role())
        .filter_map(|node| node.doc.as_deref())
        .map(|document| {
            let rendered = render_template_document_path(document, container);
            VirtualPath::worktree(rendered).map_err(Into::into)
        })
        .collect::<Result<Vec<_>>>()?;
    let already_applied =
        find_captured_breakdown(&template, &config.templates.roles, container, &issues).is_some();
    if !already_applied {
        paths.extend(
            (0..template.nodes.len())
                .map(|index| {
                    VirtualPath::data(format!(
                        "issues/{}.json",
                        context.identifier_at(index as u64)
                    ))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?,
        );
    }
    paths.extend(template_document_parent_paths(&paths)?);
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn render_template_document_path(template: &str, container: &Issue) -> String {
    let hard_criteria = container
        .description
        .lines()
        .map(str::trim)
        .map(|line| line.trim_start_matches(['-', '*', '+']).trim())
        .filter(|line| line.starts_with("[hard]"))
        .collect::<Vec<_>>()
        .join("\n");
    template
        .replace("{container.id}", &container.id)
        .replace("{container.short_id}", &container.short_id())
        .replace("{container.title}", &container.title)
        .replace("{container.hard_criteria}", &hard_criteria)
}

fn template_document_parent_paths(paths: &[VirtualPath]) -> Result<Vec<VirtualPath>> {
    let mut parents = Vec::new();
    for path in paths
        .iter()
        .filter(|path| path.root_class() == crate::repository_state::RepositoryRootClass::Worktree)
    {
        let mut parent = path.relative().as_path().parent();
        while let Some(relative) = parent {
            if relative.as_os_str().is_empty() {
                break;
            }
            parents.push(VirtualPath::worktree(relative)?);
            parent = relative.parent();
        }
    }
    Ok(parents)
}

fn template_config_from_image(image: &RepositoryImage) -> Result<crate::config::JitConfig> {
    let config_path = VirtualPath::CONFIG;
    let mut config = match image.file_bytes(&config_path)? {
        Some(_) => crate::repository_state::assemble_config(image)?,
        None => toml::from_str("")?,
    };
    let hierarchy_types = config
        .type_hierarchy
        .as_ref()
        .map(|hierarchy| {
            hierarchy
                .types
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    config.templates = match image.file_bytes(&VirtualPath::TEMPLATES)? {
        Some(bytes) => crate::templates::TemplateRegistry::from_toml_str(
            std::str::from_utf8(bytes)?,
            &hierarchy_types,
        )?,
        None => crate::templates::TemplateRegistry::empty(),
    };
    Ok(config)
}

fn template_declarations_from_image(
    image: &RepositoryImage,
    config: &crate::config::JitConfig,
) -> Result<crate::repository_state::CapturedRepositoryDeclarations> {
    if image.file_bytes(&VirtualPath::CONFIG)?.is_some() {
        return Ok(crate::repository_state::declarations_from_image(image)?);
    }
    let namespaces = crate::config_manager::namespaces_from_config(config);
    let rules = match image.file_bytes(&VirtualPath::RULES)? {
        Some(bytes) => {
            let content = std::str::from_utf8(bytes)?;
            let schemas: Vec<_> = crate::declarations::rules::RuleSet::schema_requests(content)?
                .into_iter()
                .map(|request| {
                    let path = VirtualPath::data(&request.reference)?;
                    let bytes = image.file_bytes(&path)?.ok_or_else(|| {
                        anyhow!("captured rule schema '{}' is absent", request.reference)
                    })?;
                    Ok((request.reference, bytes.to_vec()))
                })
                .collect::<Result<Vec<_>>>()?;
            let parsed =
                crate::declarations::rules::RuleSet::parse(content, Some(config), schemas)?;
            crate::repository_state::reconcile_default_rules_with_config(parsed, &namespaces)
        }
        None => crate::repository_state::default_ruleset(&namespaces),
    };
    Ok(
        crate::repository_state::CapturedRepositoryDeclarations::from_parts(
            crate::declarations::parse_configuration(b"")?,
            config.clone(),
            parse_template_gate_registry(image)?,
            rules,
        ),
    )
}

fn parse_template_index(
    image: &RepositoryImage,
) -> Result<crate::repository_state::RepositoryIndex> {
    image
        .file_bytes(&VirtualPath::INDEX)?
        .ok_or_else(|| anyhow!("index.json is absent during template apply"))
        .and_then(crate::storage::json::parse_repository_index)
}

fn listed_json_paths(image: &RepositoryImage, dir: &VirtualPath) -> Result<Vec<VirtualPath>> {
    let Some(listing) = image.listing_fingerprints().get(dir) else {
        return Err(anyhow!("complete listing is absent for {dir:?}"));
    };
    listing
        .children()
        .keys()
        .filter(|name| name.ends_with(".json"))
        .map(|name| {
            VirtualPath::data(format!("{}/{}", dir.relative().as_str(), name)).map_err(Into::into)
        })
        .collect()
}

fn parse_template_issues(image: &RepositoryImage) -> Result<Vec<Issue>> {
    let index = parse_template_index(image)?;
    let dir = VirtualPath::ISSUES;
    let listing = image
        .listing_fingerprints()
        .get(&dir)
        .ok_or_else(|| anyhow!("complete issues listing is absent from template capture"))?;
    let indexed = index
        .all_ids
        .iter()
        .map(|id| format!("{id}.json"))
        .collect::<BTreeSet<_>>();
    let listed = listing
        .children()
        .keys()
        .filter(|name| name.ends_with(".json"))
        .cloned()
        .collect::<BTreeSet<_>>();
    if indexed != listed {
        return Err(anyhow!(
            "issues directory membership does not match captured index"
        ));
    }
    index
        .all_ids
        .iter()
        .map(|id| {
            let path = VirtualPath::data(format!("issues/{id}.json"))?;
            let bytes = image
                .file_bytes(&path)?
                .ok_or_else(|| IssueNotFoundError::new(id))?;
            let issue: Issue = serde_json::from_slice(bytes)
                .with_context(|| format!("failed to parse captured issue {id}"))?;
            if issue.id != *id {
                return Err(anyhow!(
                    "indexed issue {id} contains mismatched embedded id {}",
                    issue.id
                ));
            }
            Ok(issue)
        })
        .collect()
}

fn parse_template_gate_registry(
    image: &RepositoryImage,
) -> Result<crate::declarations::GateRegistry> {
    match image.entry(&VirtualPath::GATES)? {
        RepositoryEntry::File { bytes, .. } => Ok(crate::declarations::parse_gate_registry(bytes)?),
        RepositoryEntry::Absent => Ok(crate::declarations::GateRegistry::default()),
        _ => Err(anyhow!("captured gate registry is not an ordinary file")),
    }
}

fn parse_template_presets(
    image: &RepositoryImage,
) -> Result<HashMap<String, crate::gate_presets::GatePresetDefinition>> {
    let dir = VirtualPath::GATE_PRESETS;
    let files = listed_json_paths(image, &dir)?
        .into_iter()
        .map(|path| {
            let bytes = image
                .file_bytes(&path)?
                .ok_or_else(|| anyhow!("listed custom preset is absent: {path:?}"))?;
            Ok((path.relative().as_str().to_string(), bytes.to_vec()))
        })
        .collect::<Result<Vec<_>>>()?;
    crate::gate_presets::load_presets_from_custom_files(files).map(|(presets, _)| presets)
}

#[allow(clippy::too_many_arguments)]
fn derive_template_apply(
    template: &GraphTemplate,
    container_id: &str,
    anchor_bindings: &BTreeMap<String, String>,
    force: bool,
    roles: &RoleBindings,
    issues: &[Issue],
    mut registry: crate::declarations::GateRegistry,
    presets: &HashMap<String, crate::gate_presets::GatePresetDefinition>,
    config: &crate::config::JitConfig,
    rules: &crate::declarations::rules::RuleSet,
    context: &MutationContext,
) -> Result<DerivedTemplateApply> {
    let original_registry = registry.clone();
    let full_container_id = resolve_template_issue_id(issues, container_id)?;
    let container = template_issue(issues, &full_container_id)?.clone();
    match label_utils::type_label_value(&container.labels) {
        Some(ty) if template.applies_to.iter().any(|allowed| allowed == ty) => {}
        Some(ty) => {
            return Err(anyhow!(
                "template '{}' does not apply to container type '{ty}'; applies_to: {}",
                template.name,
                template.applies_to.join(", ")
            ))
        }
        None => {
            return Err(anyhow!(
                "container {full_container_id} has no type: label; template '{}' applies to: {}",
                template.name,
                template.applies_to.join(", ")
            ))
        }
    }

    let resolved_bindings = template
        .anchors
        .iter()
        .map(|anchor| {
            let bound = anchor_bindings.get(&anchor.name).ok_or_else(|| {
                anyhow!(
                    "template '{}' anchor '{}' is not bound; bind it with --anchor {}=<id>",
                    template.name,
                    anchor.name,
                    anchor.name
                )
            })?;
            let full = resolve_template_issue_id(issues, bound).with_context(|| {
                format!(
                    "template '{}' anchor '{}' is bound to '{bound}', which does not resolve to an existing issue",
                    template.name, anchor.name
                )
            })?;
            Ok((anchor.name.clone(), full))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let anchor_dependency_snapshots = resolved_bindings
        .iter()
        .map(|(name, id)| {
            Ok((
                name.clone(),
                template_issue(issues, id)?.dependencies.clone(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;

    let existing_breakdown = find_captured_breakdown(template, roles, &container, issues);
    if existing_breakdown.is_some() && !force {
        return Err(anyhow!(
            "container {full_container_id} already has template '{}' applied; pass --force to refresh the existing nodes in place",
            template.name
        ));
    }
    if existing_breakdown.is_none() {
        if let Some(planning_type) = template.planning_type(roles) {
            if let Some(planning) = container.dependencies.iter().find_map(|id| {
                template_issue(issues, id).ok().filter(|issue| {
                    label_utils::type_label_value(&issue.labels) == Some(planning_type)
                })
            }) {
                return Err(anyhow!(
                    "container {full_container_id} already has a planning node ({}) but no breakdown node — a legacy P-only bracket. Applying '{}' would create a duplicate planning node. Remove the legacy planning node and its container edge first, then re-apply.",
                    planning.short_id(),
                    template.name
                ));
            }
        }
    }

    let mut events = Vec::new();
    let FreshTemplateDerivation {
        created: created_node_ids_by_role,
        candidate,
        created_order,
        lease_targets,
    } = if let Some(breakdown_id) = existing_breakdown {
        let (mapping, updates) = derive_template_refresh(
            template,
            roles,
            &breakdown_id,
            &container,
            issues,
            &mut events,
        )?;
        FreshTemplateDerivation {
            created: mapping,
            candidate: updates,
            created_order: Vec::new(),
            lease_targets: Vec::new(),
        }
    } else {
        // The declarations a node's document area is resolved against, read from
        // the same captured configuration as the rest of the derivation.
        let documentation = config.documentation.clone().unwrap_or_default();
        let hierarchy = crate::repository_state::hierarchy_config(
            &crate::config_manager::namespaces_from_config(config),
        );
        let delta = expand_template(
            template,
            &container,
            &resolved_bindings,
            &anchor_dependency_snapshots,
            &documentation,
            &hierarchy,
        )?;
        prevalidate_captured_delta(template, &delta, issues)?;
        derive_fresh_template(
            template,
            &delta,
            issues,
            &mut registry,
            presets,
            config,
            rules,
            context,
            &mut events,
        )?
    };

    let original = issues
        .iter()
        .map(|issue| (issue.id.as_str(), issue))
        .collect::<HashMap<_, _>>();
    let created_ids = created_order.iter().cloned().collect::<HashSet<_>>();
    let mut intents = created_order
        .iter()
        .map(|id| {
            Ok(MutationIntent::CreateIssue {
                draft: Box::new(candidate.get(id).cloned().ok_or_else(|| {
                    anyhow!("internal error: missing finalized template node {id}")
                })?),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    intents.extend(
        candidate
            .values()
            .filter(|issue| {
                !created_ids.contains(&issue.id)
                    && original
                        .get(issue.id.as_str())
                        .is_some_and(|old| *old != *issue)
            })
            .cloned()
            .map(|issue| MutationIntent::UpdateIssue {
                issue: Box::new(issue),
            }),
    );
    if registry != original_registry {
        intents.push(MutationIntent::EditGateRegistry {
            registry: Box::new(registry.clone()),
        });
    }
    intents.extend(
        events
            .into_iter()
            .map(|(phase, event)| MutationIntent::RecordEvent {
                phase,
                event: Box::new(event),
            }),
    );

    Ok(DerivedTemplateApply {
        result: TemplateApplyResult {
            template: template.name.clone(),
            anchor_bindings: resolved_bindings,
            created_node_ids_by_role,
            anchor_dependency_snapshots,
        },
        registry,
        intents,
        lease_targets,
    })
}

fn resolve_template_issue_id(issues: &[Issue], partial: &str) -> Result<String> {
    let normalized = partial.to_lowercase().replace('-', "");
    if normalized.len() < MIN_ID_PREFIX_LENGTH {
        return Err(InvalidIdPrefixError::new(partial).into());
    }
    let matches = issues
        .iter()
        .filter(|issue| {
            issue
                .id
                .replace('-', "")
                .to_lowercase()
                .starts_with(&normalized)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(IssueNotFoundError::new(partial).into()),
        [issue] => Ok(issue.id.clone()),
        _ => Err(AmbiguousIdError::issue(
            partial,
            matches
                .iter()
                .map(|issue| format!("{} | {}", issue.short_id(), issue.title)),
        )
        .into()),
    }
}

fn template_issue<'a>(issues: &'a [Issue], id: &str) -> Result<&'a Issue> {
    issues
        .iter()
        .find(|issue| issue.id == id)
        .ok_or_else(|| IssueNotFoundError::new(id).into())
}

fn find_captured_breakdown(
    template: &GraphTemplate,
    roles: &RoleBindings,
    container: &Issue,
    issues: &[Issue],
) -> Option<String> {
    let breakdown = template.breakdown_node(roles)?;
    let bracket = format!("brackets:{}", container.short_id());
    issues
        .iter()
        .find(|issue| {
            label_utils::type_label_value(&issue.labels) == Some(breakdown.type_name.as_str())
                && issue.labels.contains(&bracket)
        })
        .map(|issue| issue.id.clone())
}

fn prevalidate_captured_delta(
    template: &GraphTemplate,
    delta: &TemplateDelta,
    issues: &[Issue],
) -> Result<()> {
    let dependencies = issues
        .iter()
        .map(|issue| (issue.id.clone(), issue.dependencies.clone()))
        .collect();
    validate_delta_acyclic(delta, dependencies).map_err(|error| {
        error.context(format!(
            "applying template '{}' would create a dependency cycle; no nodes were created",
            template.name
        ))
    })
}

fn derive_template_refresh(
    template: &GraphTemplate,
    roles: &RoleBindings,
    breakdown_id: &str,
    container: &Issue,
    issues: &[Issue],
    events: &mut Vec<(u8, Event)>,
) -> Result<(BTreeMap<String, String>, BTreeMap<String, Issue>)> {
    let context = InterpolationContext::for_container(container);
    let mut mapping =
        BTreeMap::from([(roles.breakdown_role().to_string(), breakdown_id.to_string())]);
    if let Some(node) = template.breakdown_node(roles) {
        let breakdown = template_issue(issues, breakdown_id)?;
        for role in &node.depends_on {
            let Some(dependency_node) = template.node(role) else {
                continue;
            };
            if let Some(issue) = breakdown.dependencies.iter().find_map(|id| {
                template_issue(issues, id).ok().filter(|issue| {
                    label_utils::type_label_value(&issue.labels)
                        == Some(dependency_node.type_name.as_str())
                })
            }) {
                mapping.insert(role.clone(), issue.id.clone());
            }
        }
    }
    if let Some(missing) = template
        .nodes
        .iter()
        .find(|node| !mapping.contains_key(&node.role))
    {
        return Err(anyhow!(
            "cannot --force refresh template '{}': its '{}' node could not be located from the existing bracket (the applied bracket is broken or incomplete); the bracket must be repaired before it can be refreshed",
            template.name,
            missing.role
        ));
    }
    let mut updates = BTreeMap::new();
    for node in &template.nodes {
        let id = mapping.get(&node.role).ok_or_else(|| {
            anyhow!(
                "internal error: template '{}' role '{}' vanished during refresh",
                template.name,
                node.role
            )
        })?;
        let mut issue = template_issue(issues, id)?.clone();
        // Prose reconciliation: the artifact-directory field is out of scope here.
        let description = node_description(node, &context.with_doc(node, None));
        if issue.description == description {
            continue;
        }
        issue.description = description;
        updates.insert(id.clone(), issue);
        events.push((
            1,
            Event::draft_issue_updated(
                id.clone(),
                APPLY_ACTOR.to_string(),
                vec!["description".to_string()],
            ),
        ));
    }
    Ok((mapping, updates))
}

#[allow(clippy::too_many_arguments)]
fn derive_fresh_template(
    template: &GraphTemplate,
    delta: &TemplateDelta,
    issues: &[Issue],
    registry: &mut crate::declarations::GateRegistry,
    presets: &HashMap<String, crate::gate_presets::GatePresetDefinition>,
    config: &crate::config::JitConfig,
    rules: &crate::declarations::rules::RuleSet,
    context: &MutationContext,
    events: &mut Vec<(u8, Event)>,
) -> Result<FreshTemplateDerivation> {
    let created_order = (0..delta.creates.len())
        .map(|index| context.identifier_at(index as u64))
        .collect::<Vec<_>>();
    let created = delta
        .creates
        .iter()
        .zip(&created_order)
        .map(|(planned, id)| (planned.role.clone(), id.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut candidate = issues
        .iter()
        .cloned()
        .map(|issue| (issue.id.clone(), issue))
        .collect::<BTreeMap<_, _>>();
    let mut lease_targets = Vec::new();

    for (planned, id) in delta.creates.iter().zip(&created_order) {
        let mut issue = project_planned_issue(planned);
        issue.id = id.clone();
        attach_captured_gates(
            &planned.gates,
            &mut issue,
            registry,
            presets,
            events,
            &mut lease_targets,
        )
        .with_context(|| {
            format!(
                "template '{}' node '{}' references invalid gate(s): {}",
                template.name,
                planned.role,
                planned.gates.join(", ")
            )
        })?;
        validate_captured_issue_write(&issue, config, rules).map_err(|_| {
            crate::errors::InvalidArgumentError::new(format!(
                "template '{}' node '{}' would create an invalid issue",
                template.name, planned.role
            ))
        })?;
        candidate.insert(id.clone(), issue);
    }

    for edge in &delta.add_edges {
        add_captured_template_edge(
            &mut candidate,
            &resolve_endpoint(&edge.dependent, &created)?,
            &resolve_endpoint(&edge.dependency, &created)?,
            events,
            &mut lease_targets,
        )?;
    }
    for edge in &delta.remove_edges {
        remove_captured_template_edge(
            &mut candidate,
            &resolve_endpoint(&edge.dependent, &created)?,
            &resolve_endpoint(&edge.dependency, &created)?,
            events,
            &mut lease_targets,
        )?;
    }
    for attachment in &delta.anchor_gates {
        let issue = candidate
            .get_mut(&attachment.anchor_issue_id)
            .ok_or_else(|| IssueNotFoundError::new(&attachment.anchor_issue_id))?;
        attach_captured_gates(
            &attachment.gates,
            issue,
            registry,
            presets,
            events,
            &mut lease_targets,
        )
        .with_context(|| {
            format!(
                "template '{}' anchor references invalid gate(s): {}",
                template.name,
                attachment.gates.join(", ")
            )
        })?;
    }
    Ok(FreshTemplateDerivation {
        created,
        candidate,
        created_order,
        lease_targets,
    })
}

fn validate_captured_issue_write(
    issue: &Issue,
    config: &crate::config::JitConfig,
    rules: &crate::declarations::rules::RuleSet,
) -> Result<()> {
    let format = match config.validation.as_ref() {
        Some(validation) => validation.content_format()?,
        None => crate::domain::ContentFormat::Markdown,
    };
    let strictness = match config.validation.as_ref() {
        Some(validation) => validation.strictness()?,
        None => crate::validation::Strictness::Loose,
    };
    let evaluation = crate::validation::evaluate_local(issue, rules, format)
        .map_err(|error| anyhow!("rule evaluation failed: {error}"))?
        .with_strictness(strictness);
    if evaluation.blocking_rules().is_empty() {
        Ok(())
    } else {
        Err(crate::errors::ValidationFailedError::new(
            evaluation
                .rejection_message()
                .unwrap_or_else(|| "blocked by validation rule(s)".to_string()),
        )
        .into())
    }
}

fn resolve_captured_gate(
    name: &str,
    registry: &crate::declarations::GateRegistry,
    presets: &HashMap<String, crate::gate_presets::GatePresetDefinition>,
) -> Result<TemplateGateResolution> {
    if let Some(preset) = presets.get(name) {
        return Ok(TemplateGateResolution::Preset(preset.clone()));
    }
    if registry.gates.contains_key(name) {
        return Ok(TemplateGateResolution::RegistryKey);
    }
    Err(anyhow!(
        "gate '{name}' is neither a registered gate preset nor a gate defined in the registry"
    ))
}

fn attach_captured_gates(
    names: &[String],
    issue: &mut Issue,
    registry: &mut crate::declarations::GateRegistry,
    presets: &HashMap<String, crate::gate_presets::GatePresetDefinition>,
    events: &mut Vec<(u8, Event)>,
    lease_targets: &mut Vec<String>,
) -> Result<()> {
    for name in names {
        let keys = match resolve_captured_gate(name, registry, presets)? {
            TemplateGateResolution::Preset(preset) => {
                // The legacy preset path checked once before registry edits and
                // once again when attaching the resulting keys.
                lease_targets.extend([issue.id.clone(), issue.id.clone()]);
                preset
                    .gates
                    .into_iter()
                    .map(|template| {
                        let gate = template.to_gate();
                        if !registry.gates.contains_key(&gate.key) {
                            events
                                .push((0, Event::draft_gate_definition_created(gate.key.clone())));
                            registry.gates.insert(gate.key.clone(), gate.clone());
                        }
                        gate.key
                    })
                    .collect::<Vec<_>>()
            }
            TemplateGateResolution::RegistryKey => {
                lease_targets.push(issue.id.clone());
                vec![name.clone()]
            }
        };
        for key in keys {
            if issue.gates_required.contains(&key) {
                continue;
            }
            issue.gates_required.push(key.clone());
            issue.gates_status.insert(
                key.clone(),
                GateState {
                    status: GateStatus::Pending,
                    updated_by: None,
                    updated_at: chrono::DateTime::default(),
                },
            );
            events.push((1, Event::draft_gate_added(issue.id.clone(), key)));
        }
    }
    Ok(())
}

fn add_captured_template_edge(
    issues: &mut BTreeMap<String, Issue>,
    from: &str,
    to: &str,
    events: &mut Vec<(u8, Event)>,
    lease_targets: &mut Vec<String>,
) -> Result<()> {
    lease_targets.push(from.to_string());
    let refs = issues.values().collect::<Vec<_>>();
    DependencyGraph::new(&refs).validate_add_dependency(from, to)?;
    let before = issues
        .get(from)
        .ok_or_else(|| IssueNotFoundError::new(from))?
        .dependencies
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    if before.contains(to) {
        return Ok(());
    }
    issues
        .get_mut(from)
        .ok_or_else(|| IssueNotFoundError::new(from))?
        .dependencies
        .push(to.to_string());

    let reductions = {
        let refs = issues.values().collect::<Vec<_>>();
        let graph = DependencyGraph::new(&refs);
        issues
            .keys()
            .map(|id| (id.clone(), graph.compute_transitive_reduction(id)))
            .collect::<BTreeMap<_, _>>()
    };
    for (id, reduced) in reductions {
        let newly_added = if id == from {
            reduced.difference(&before).cloned().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let blocks = newly_added.iter().any(|dependency| {
            issues.get(dependency).is_some_and(|dependency| {
                !is_dependency_met(dependency.state, dependency.archived_from)
            })
        });
        let issue = issues
            .get_mut(&id)
            .ok_or_else(|| IssueNotFoundError::new(&id))?;
        let old = issue.dependencies.iter().cloned().collect::<HashSet<_>>();
        if old == reduced {
            continue;
        }
        let mut removed = old.difference(&reduced).cloned().collect::<Vec<_>>();
        removed.sort();
        issue.dependencies = reduced.iter().cloned().collect();
        issue.dependencies.sort();
        if id == from {
            if issue.state == State::Ready && blocks {
                issue.state = State::Backlog;
                events.push((
                    1,
                    Event::draft_issue_state_changed(id.clone(), State::Ready, State::Backlog),
                ));
            }
            events.push((
                2,
                Event::draft_issue_updated(
                    id,
                    "dependency-add".to_string(),
                    vec!["dependencies".to_string()],
                ),
            ));
        } else {
            events.push((
                3,
                Event::draft_dependency_reduced(id, old.len(), reduced.len(), removed),
            ));
        }
    }
    Ok(())
}

fn remove_captured_template_edge(
    issues: &mut BTreeMap<String, Issue>,
    from: &str,
    to: &str,
    events: &mut Vec<(u8, Event)>,
    lease_targets: &mut Vec<String>,
) -> Result<()> {
    lease_targets.push(from.to_string());
    let remaining = {
        let issue = issues
            .get_mut(from)
            .ok_or_else(|| IssueNotFoundError::new(from))?;
        let before = issue.dependencies.len();
        issue.dependencies.retain(|dependency| dependency != to);
        if issue.dependencies.len() == before {
            return Ok(());
        }
        issue.dependencies.clone()
    };
    events.push((
        2,
        Event::draft_issue_updated(
            from.to_string(),
            "dependency-remove".to_string(),
            vec!["dependencies".to_string()],
        ),
    ));
    let all_met = remaining.iter().all(|dependency| {
        issues
            .get(dependency)
            .is_some_and(|dependency| is_dependency_met(dependency.state, dependency.archived_from))
    });
    let issue = issues
        .get_mut(from)
        .ok_or_else(|| IssueNotFoundError::new(from))?;
    let ready = issue.state == State::Backlog && all_met;
    if ready {
        issue.state = State::Ready;
        events.push((
            3,
            Event::draft_issue_state_changed(from.to_string(), State::Backlog, State::Ready),
        ));
    }
    Ok(())
}

/// The issue id a [`DeltaEndpoint`] names: the id created for its role, or the
/// existing id it carries.
fn resolve_endpoint(
    endpoint: &DeltaEndpoint,
    created: &BTreeMap<String, String>,
) -> Result<String> {
    match endpoint {
        DeltaEndpoint::CreatedRole(role) => created.get(role).cloned().ok_or_else(|| {
            anyhow!("internal error: no issue was created for template role '{role}'")
        }),
        DeltaEndpoint::ExistingIssue(id) => Ok(id.clone()),
    }
}

/// Build the FINAL persisted [`Issue`] shape a planned node's `create_issue` would
/// produce, for read-only pre-validation. Mirrors `create_issue`'s construction
/// for a node that always carries a `type:` label and has no dependencies at
/// creation: fields set, then auto-promoted to [`State::Ready`] (so a state-keyed
/// rule sees the persisted shape).
fn project_planned_issue(planned: &PlannedNode) -> Issue {
    let mut issue = Issue::draft(planned.title.clone(), planned.description.clone());
    issue.priority = planned.priority;
    issue.labels = planned.labels.clone();
    // A freshly-created issue with no dependencies is auto-promoted to Ready
    // by `create_issue`; replicate so state-keyed rules see the same shape.
    issue.state = State::Ready;
    issue
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_label_value_extracts_type() {
        let mut issue = crate::domain::types::fixture_issue("T".to_string(), String::new());
        issue.labels = vec!["type:epic".to_string(), "area:auth".to_string()];
        assert_eq!(label_utils::type_label_value(&issue.labels), Some("epic"));
    }

    #[test]
    fn test_project_planned_issue_mirrors_created_shape() {
        let planned = PlannedNode {
            role: "planning".to_string(),
            title: "planning: Epic".to_string(),
            description: "Plan it.".to_string(),
            labels: vec!["type:planning".to_string()],
            gates: vec![],
            priority: Priority::High,
        };
        let projected = project_planned_issue(&planned);
        assert_eq!(projected.title, "planning: Epic");
        assert_eq!(projected.priority, Priority::High);
        assert_eq!(projected.state, State::Ready);
        assert!(projected.dependencies.is_empty());
    }

    #[test]
    fn test_resolve_endpoint_maps_roles_and_passes_ids_through() {
        let created = BTreeMap::from([("planning".to_string(), "p1".to_string())]);
        assert_eq!(
            resolve_endpoint(
                &DeltaEndpoint::CreatedRole("planning".to_string()),
                &created
            )
            .unwrap(),
            "p1"
        );
        assert_eq!(
            resolve_endpoint(&DeltaEndpoint::ExistingIssue("c1".to_string()), &created).unwrap(),
            "c1"
        );
        assert!(
            resolve_endpoint(&DeltaEndpoint::CreatedRole("missing".to_string()), &created).is_err()
        );
    }
}
