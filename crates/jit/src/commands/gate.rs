//! Quality gate operations

use super::*;
use crate::declarations::GateMode;
use crate::domain::{GateRunResult, GateRunStatus};

/// Error returned when `jit gate evaluate` runs an automated checker that does not pass.
///
/// The checker result is preserved so CLI callers can report the failed status
/// without disagreeing with the persisted `gates_status` value.
#[derive(Debug, thiserror::Error)]
#[error(
    "Gate '{gate_key}' failed for issue {issue_id}. Checker status: {status:?}, exit code: {exit_code:?}. Inspect details with: jit gate status {issue_id} {gate_key} (add --all for run history, --findings for structured findings)"
)]
pub struct GatePassFailed {
    /// Issue whose gate was checked.
    pub issue_id: String,
    /// Gate key that failed.
    pub gate_key: String,
    /// Raw checker status.
    pub status: GateRunStatus,
    /// Checker process exit code, when available.
    pub exit_code: Option<i32>,
    /// Full checker result persisted under `.jit/gate-runs/`.
    pub result: GateRunResult,
    /// Warnings gathered before running the checker.
    pub warnings: Vec<String>,
}

/// Error returned when `jit gate evaluate` targets a gate that the issue does not require.
///
/// This is an argument/lookup error raised before any checker runs: the named
/// gate is simply not in the issue's `gates_required` list. CLI callers classify
/// it as an invalid-argument condition (exit code `2`), distinct from a checker
/// failure or a runner error.
#[derive(Debug, thiserror::Error)]
#[error("Gate '{gate_key}' is not required for issue {issue_id}")]
pub struct GateNotRequiredError {
    /// Issue that was targeted.
    pub issue_id: String,
    /// Gate key that is not in the issue's required set.
    pub gate_key: String,
}

/// Error returned when `jit gate evaluate`/`pass` targets a MANUAL gate with no
/// `--by` attestor.
///
/// A manual gate has no checker to run, so its "evaluation" is a human
/// attestation — recording it without saying who attested is indistinguishable
/// from a silent, unverified pass (the exact failure mode this type closes:
/// jit:1d59070d). This is an argument/usage error (exit code `2`, the
/// same family as an explicit `--mode manual` + `--checker-command` conflict
/// on `gate define`), raised before any write happens. Automated gates are
/// unaffected — their verdict comes from the checker, not `--by`.
#[derive(Debug, thiserror::Error)]
#[error(
    "Gate '{gate_key}' is manual and requires an attestor. Record the pass with: jit gate evaluate {issue_id} {gate_key} --by <attestor>"
)]
pub struct ManualGateAttestationRequiredError {
    /// Issue whose gate was targeted.
    pub issue_id: String,
    /// Manual gate key that was targeted without `--by`.
    pub gate_key: String,
}

/// Outcome of a successful [`pass_gate`](CommandExecutor::pass_gate) call.
///
/// Carries any warnings gathered along the way (e.g. lease warnings) plus
/// `already_passed`, which is `true` when the gate was found to have already
/// passed at the current `HEAD` commit and the (potentially expensive) checker
/// was skipped. On a normal run — manual attestation or a freshly executed
/// checker — `already_passed` is `false`.
#[derive(Debug, Clone)]
pub struct GatePassOutcome {
    /// Warnings gathered while passing the gate (e.g. lease warnings).
    pub warnings: Vec<String>,
    /// `true` when the checker was skipped because the gate already passed at
    /// the current `HEAD` commit.
    pub already_passed: bool,
}

/// Per-gate entry in a [`PassAllOutcome`].
///
/// Records which gate was passed and whether its checker actually ran
/// (`already_passed == false`) or was skipped because it already passed at the
/// current `HEAD` (`already_passed == true`), plus any warnings gathered.
#[derive(Debug, Clone)]
pub struct GatePassAllEntry {
    /// The gate that was passed.
    pub gate_key: String,
    /// `true` when the checker was skipped (gate already passed at `HEAD`).
    pub already_passed: bool,
    /// Warnings gathered while passing this gate.
    pub warnings: Vec<String>,
}

/// Outcome of a successful [`pass_all_gates`](CommandExecutor::pass_all_gates) run.
///
/// `results` holds one [`GatePassAllEntry`] per required gate, in declaration
/// order. When the issue has no required gates the list is empty (the command
/// still succeeds with exit `0`). On the first non-passing gate `pass_all_gates`
/// fails fast and returns the underlying error instead of this outcome, so a
/// `PassAllOutcome` always describes an all-green run.
#[derive(Debug, Clone)]
pub struct PassAllOutcome {
    /// Per-gate results, in `gates_required` order.
    pub results: Vec<GatePassAllEntry>,
}

/// The outcome of evaluating one gate across several issues.
pub struct GateEvaluateManyEntry {
    /// Issue identifier as supplied to the command.
    pub issue_id: String,
    /// The per-issue gate result, or the error that prevented a verdict.
    pub result: Result<GatePassOutcome>,
}

/// Per-issue results from [`CommandExecutor::pass_gate_many`].
pub struct GateEvaluateManyOutcome {
    /// Results in the same order as the requested issue identifiers.
    pub results: Vec<GateEvaluateManyEntry>,
}

/// Result of adding multiple gates
#[derive(Debug, Serialize)]
pub struct GateAddResult {
    pub added: Vec<String>,
    pub already_exist: Vec<String>,
}

/// Result of removing multiple gates
#[derive(Debug, Serialize)]
pub struct GateRemoveResult {
    pub removed: Vec<String>,
    pub not_found: Vec<String>,
}

/// A tri-state edit for an optional/clearable field.
///
/// Distinguishes "leave the field as it is" from "clear the field", which a
/// plain `Option<T>` cannot: `Some(v)` would set and `None` is ambiguous
/// between "unchanged" and "remove". Used by [`GateUpdate`] for the fields that
/// `jit gate update` must be able to both set AND clear (working dir, prompt,
/// prompt file, checker env) so editing never requires hand-editing the
/// registry file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum FieldEdit<T> {
    /// Leave the field unchanged.
    #[default]
    Keep,
    /// Replace the field with a new value.
    Set(T),
    /// Clear the field (set it to none / empty).
    Clear,
}

impl<T> FieldEdit<T> {
    /// `true` when this edit leaves the field unchanged.
    pub fn is_keep(&self) -> bool {
        matches!(self, FieldEdit::Keep)
    }
}

/// Mutable fields for [`update_gate`](CommandExecutor::update_gate).
///
/// Each `Some`/`Set` field replaces the gate's current value; each `None`/`Keep`
/// leaves it unchanged. The clearable fields ([`FieldEdit`]) additionally
/// support `Clear`, so every mutable field is reachable from the CLI without
/// hand-editing the registry. The gate KEY is the gate's identity and is
/// therefore not part of this struct. The checker fields (`checker_command`,
/// `timeout`, `working_dir`, `pass_context`, `prompt`, `prompt_file`, `env`)
/// are merged onto an existing exec checker. Editing one of those fields on a
/// built-in checker replaces it with an exec checker; otherwise an unprovided
/// checker field is preserved.
#[derive(Debug, Default, Clone)]
pub struct GateUpdate {
    /// New human-readable title.
    pub title: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New gate stage.
    pub stage: Option<crate::declarations::GateStage>,
    /// New gate mode (manual or auto).
    pub mode: Option<crate::declarations::GateMode>,
    /// New execution priority.
    pub priority: Option<u32>,
    /// New checker command.
    pub checker_command: Option<String>,
    /// New checker timeout in seconds.
    pub timeout: Option<u64>,
    /// New checker working directory (set or clear).
    pub working_dir: FieldEdit<String>,
    /// New `pass_context` flag for the checker.
    pub pass_context: Option<bool>,
    /// New inline prompt for the checker (set or clear).
    pub prompt: FieldEdit<String>,
    /// New prompt-file path for the checker (set or clear).
    pub prompt_file: FieldEdit<String>,
    /// Replacement environment set for the checker (set replaces the whole map;
    /// clear empties it).
    pub env: FieldEdit<std::collections::HashMap<String, String>>,
}

impl GateUpdate {
    /// `true` when no field is set, i.e. the update would change nothing.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.stage.is_none()
            && self.mode.is_none()
            && self.priority.is_none()
            && self.checker_command.is_none()
            && self.timeout.is_none()
            && self.working_dir.is_keep()
            && self.pass_context.is_none()
            && self.prompt.is_keep()
            && self.prompt_file.is_keep()
            && self.env.is_keep()
    }
}

#[derive(Clone)]
enum CapturedGateRegistryMutation {
    Define(GateDefinition),
    Update { key: String, update: GateUpdate },
    Remove { key: String },
}

enum CapturedGateRegistryOutcome {
    Defined,
    Updated(Box<GateDefinition>),
    Removed,
}

struct DerivedGateRegistryMutation {
    registry: crate::declarations::GateRegistry,
    event: Event,
    outcome: CapturedGateRegistryOutcome,
}

struct DerivedGatePresetApplication {
    registry: crate::declarations::GateRegistry,
    intents: Vec<crate::repository_state::MutationIntent>,
    result: GateAddResult,
    edits_registry: bool,
}

const GATE_PRESET_CAPTURE_BUDGET: crate::repository_state::CaptureBudget =
    crate::repository_state::CaptureBudget {
        max_listings: 64,
        max_bytes: 512 * 1024 * 1024,
        max_depth: 16,
    };

fn listed_gate_preset_paths(
    image: &crate::repository_state::RepositoryImage,
) -> Result<Vec<crate::repository_state::VirtualPath>> {
    use crate::repository_state::VirtualPath;
    let directory = VirtualPath::GATE_PRESETS;
    let listing = image
        .listing_fingerprints()
        .get(&directory)
        .ok_or_else(|| anyhow!("complete custom gate preset listing is absent"))?;
    let prefix = directory.relative().as_str();
    listing
        .children()
        .keys()
        .filter(|name| name.ends_with(".json"))
        .map(|name| VirtualPath::data(format!("{prefix}/{name}")).map_err(Into::into))
        .collect()
}

fn parse_captured_gate_presets(
    image: &crate::repository_state::RepositoryImage,
) -> Result<std::collections::HashMap<String, crate::gate_presets::GatePresetDefinition>> {
    let files = listed_gate_preset_paths(image)?
        .into_iter()
        .map(|path| {
            let bytes = image
                .file_bytes(&path)?
                .ok_or_else(|| anyhow!("listed custom gate preset is absent: {path:?}"))?;
            Ok((path.relative().as_str().to_string(), bytes.to_vec()))
        })
        .collect::<Result<Vec<_>>>()?;
    crate::gate_presets::load_presets_from_custom_files(files)
}

fn captured_gate_preset_fixed_paths() -> Vec<crate::repository_state::VirtualPath> {
    use crate::repository_state::VirtualPath;
    vec![
        VirtualPath::CONFIG,
        VirtualPath::INDEX,
        VirtualPath::INVARIANTS,
        VirtualPath::RULES,
        VirtualPath::GATES,
        VirtualPath::EVENTS,
        VirtualPath::CONFIG_DIR,
        VirtualPath::GATE_PRESETS,
        VirtualPath::ISSUES,
    ]
}

fn capture_gate_preset_image(
    session: &mut dyn crate::storage::RepositoryMutationSession,
    include_projection_closure: bool,
    create_target: Option<&str>,
) -> Result<Option<crate::repository_state::RepositoryImage>> {
    use crate::repository_state::{
        assemble_config, render_capture_closure, CaptureSpec, VirtualPath,
    };

    let presets_dir = VirtualPath::GATE_PRESETS;
    let issues_dir = VirtualPath::ISSUES;
    let mut first_spec = CaptureSpec::phase_one(
        captured_gate_preset_fixed_paths(),
        GATE_PRESET_CAPTURE_BUDGET,
    )?;
    first_spec.discover_listing(presets_dir.clone())?;
    first_spec.discover_listing(issues_dir.clone())?;
    let Some(first) = capture_or_retry(session.capture(first_spec))? else {
        return Ok(None);
    };
    let discovered_presets = listed_gate_preset_paths(&first)?;
    let index_bytes = first
        .file_bytes(&VirtualPath::INDEX)?
        .ok_or_else(|| anyhow!("captured image has no .jit/index.json"))?;
    let discovered_index = crate::storage::json::parse_repository_index(index_bytes)?;
    let issue_paths = discovered_index
        .all_ids
        .iter()
        .map(|id| VirtualPath::data(format!("issues/{id}.json")).map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;
    let closure = if include_projection_closure {
        let config = assemble_config(&first)?;
        let rules_text = super::image_repo_bytes(&first, ".jit/rules.toml")?
            .map(String::from_utf8)
            .transpose()?;
        render_capture_closure(first.layout(), &config, &[], rules_text.as_deref())?
    } else {
        Vec::new()
    };

    let mut spec = CaptureSpec::phase_one(
        captured_gate_preset_fixed_paths(),
        GATE_PRESET_CAPTURE_BUDGET,
    )?;
    let presets_prefix = presets_dir.relative().as_str();
    let create_path = create_target
        .map(|name| VirtualPath::data(format!("{presets_prefix}/{name}.json")))
        .transpose()?;
    spec.discover_paths(
        discovered_presets
            .iter()
            .cloned()
            .chain(issue_paths)
            .chain(closure)
            .chain(create_path),
    )?;
    spec.discover_listing(presets_dir)?;
    spec.discover_listing(issues_dir.clone())?;
    let Some(image) = capture_or_retry(session.capture(spec))? else {
        return Ok(None);
    };
    if listed_gate_preset_paths(&image)? != discovered_presets {
        return Ok(None);
    }
    let current_index = image
        .file_bytes(&VirtualPath::INDEX)?
        .ok_or_else(|| anyhow!("captured image has no .jit/index.json"))
        .and_then(crate::storage::json::parse_repository_index)?;
    if current_index.schema_version != discovered_index.schema_version
        || current_index.all_ids != discovered_index.all_ids
        || current_index.deleted_ids != discovered_index.deleted_ids
    {
        return Ok(None);
    }
    let listing = image
        .listing_fingerprints()
        .get(&issues_dir)
        .ok_or_else(|| anyhow!("complete issues listing is absent"))?;
    let listed = listing
        .children()
        .keys()
        .filter(|name| name.ends_with(".json"))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let indexed = current_index
        .all_ids
        .iter()
        .map(|id| format!("{id}.json"))
        .collect::<std::collections::BTreeSet<_>>();
    if listed != indexed {
        return Ok(None);
    }
    Ok(Some(image))
}

fn captured_gate_registry(
    image: &crate::repository_state::RepositoryImage,
) -> Result<crate::declarations::GateRegistry> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};
    match image.entry(&VirtualPath::GATES)? {
        RepositoryEntry::File { bytes, .. } => {
            crate::declarations::parse_gate_registry(bytes).map_err(Into::into)
        }
        RepositoryEntry::Absent => Ok(crate::declarations::GateRegistry::default()),
        _ => Err(anyhow!("captured gate registry is not an ordinary file")),
    }
}

fn captured_gate_preset_issue(
    image: &crate::repository_state::RepositoryImage,
    issue_id: &str,
) -> Result<Issue> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};
    let path = VirtualPath::data(format!("issues/{issue_id}.json"))?;
    let issue = match image.entry(&path)? {
        RepositoryEntry::File { bytes, .. } => serde_json::from_slice::<Issue>(bytes)
            .with_context(|| format!("failed to parse captured issue {issue_id}"))?,
        RepositoryEntry::Absent => {
            return Err(crate::storage::IssueNotFoundError::new(issue_id).into())
        }
        _ => return Err(anyhow!("captured issue path is not an ordinary file")),
    };
    if issue.id != issue_id {
        return Err(anyhow!(
            "captured issue identity mismatch: requested {issue_id}, found {}",
            issue.id
        ));
    }
    Ok(issue)
}

fn derive_gate_preset_application(
    issue: Issue,
    mut registry: crate::declarations::GateRegistry,
    preset: &crate::gate_presets::GatePresetDefinition,
    timeout_override: Option<u64>,
    skip_precheck: bool,
    skip_postcheck: bool,
    except_gates: &[String],
) -> Result<DerivedGatePresetApplication> {
    use crate::declarations::{GateChecker, GateStage};
    use crate::repository_state::MutationIntent;

    let gates = preset
        .gates
        .iter()
        .filter(|gate| {
            !(skip_precheck && gate.stage == GateStage::Precheck
                || skip_postcheck && gate.stage == GateStage::Postcheck
                || except_gates.contains(&gate.key))
        })
        .collect::<Vec<_>>();
    if gates.is_empty() {
        return Err(anyhow!("No gates to apply after filtering"));
    }

    let mut intents = Vec::new();
    let mut edits_registry = false;
    for template in &gates {
        let mut gate = template.to_gate();
        if let (
            Some(timeout),
            Some(GateChecker::Exec {
                timeout_seconds, ..
            }),
        ) = (timeout_override, gate.checker.as_mut())
        {
            *timeout_seconds = timeout;
        }
        let existed = registry.gates.contains_key(&gate.key);
        if timeout_override.is_some() || !existed {
            edits_registry = true;
            registry.gates.insert(gate.key.clone(), gate.clone());
            intents.push(MutationIntent::RecordEvent {
                phase: 0,
                event: Box::new(if existed {
                    Event::draft_gate_definition_updated(gate.key)
                } else {
                    Event::draft_gate_definition_created(gate.key)
                }),
            });
        }
    }
    if edits_registry {
        intents.insert(
            0,
            MutationIntent::EditGateRegistry {
                registry: Box::new(registry.clone()),
            },
        );
    }
    let keys = gates
        .iter()
        .map(|gate| gate.key.clone())
        .collect::<Vec<_>>();
    let issue_id = issue.id.clone();
    let derived = super::derive_captured_issue_mutation(
        issue,
        &registry,
        &super::CapturedIssueMutation::AddGates {
            issue_id,
            gate_keys: keys,
        },
    )?;
    let super::CapturedIssueMutationOutcome::GatesAdded {
        added,
        already_exist,
    } = derived.outcome
    else {
        return Err(anyhow!(
            "unexpected result from captured gate preset application"
        ));
    };
    intents.extend(derived.intents);
    Ok(DerivedGatePresetApplication {
        registry,
        intents,
        result: GateAddResult {
            added,
            already_exist,
        },
        edits_registry,
    })
}

fn updated_gate_definition(current: GateDefinition, update: GateUpdate) -> Result<GateDefinition> {
    use crate::declarations::GateChecker;

    let final_mode = update.mode.unwrap_or(current.mode);
    let any_checker_field = update.checker_command.is_some()
        || update.timeout.is_some()
        || update.pass_context.is_some()
        || !update.working_dir.is_keep()
        || !update.prompt.is_keep()
        || !update.prompt_file.is_keep()
        || !update.env.is_keep();
    let merged_checker = if current.checker.is_some() || any_checker_field {
        if !any_checker_field
            && matches!(
                current.checker,
                Some(GateChecker::RepositoryValidation)
                    | Some(GateChecker::IssueValidation)
                    | Some(GateChecker::LabelTargetValidation { .. })
                    | Some(GateChecker::RuleValidation { .. })
                    | Some(GateChecker::ReviewPlaceholder)
            )
        {
            current.checker.clone()
        } else {
            let (
                mut command,
                mut timeout_seconds,
                mut working_dir,
                mut env,
                mut pass_context,
                mut prompt,
                mut prompt_file,
            ) = match current.checker.clone() {
                Some(GateChecker::Exec {
                    command,
                    timeout_seconds,
                    working_dir,
                    env,
                    pass_context,
                    prompt,
                    prompt_file,
                }) => (
                    command,
                    timeout_seconds,
                    working_dir,
                    env,
                    pass_context,
                    prompt,
                    prompt_file,
                ),
                Some(_) | None => (
                    String::new(),
                    300,
                    None,
                    std::collections::HashMap::new(),
                    false,
                    None,
                    None,
                ),
            };
            if let Some(value) = update.checker_command {
                command = value;
            }
            if let Some(value) = update.timeout {
                timeout_seconds = value;
            }
            if let Some(value) = update.pass_context {
                pass_context = value;
            }
            working_dir = match update.working_dir {
                FieldEdit::Keep => working_dir,
                FieldEdit::Set(value) => Some(value),
                FieldEdit::Clear => None,
            };
            prompt = match update.prompt {
                FieldEdit::Keep => prompt,
                FieldEdit::Set(value) => Some(value),
                FieldEdit::Clear => None,
            };
            prompt_file = match update.prompt_file {
                FieldEdit::Keep => prompt_file,
                FieldEdit::Set(value) => Some(value),
                FieldEdit::Clear => None,
            };
            env = match update.env {
                FieldEdit::Keep => env,
                FieldEdit::Set(value) => value,
                FieldEdit::Clear => std::collections::HashMap::new(),
            };
            Some(GateChecker::Exec {
                command,
                timeout_seconds,
                working_dir,
                env,
                pass_context,
                prompt,
                prompt_file,
            })
        }
    } else {
        None
    };
    let final_checker = (final_mode != GateMode::Manual)
        .then_some(merged_checker)
        .flatten();
    let has_valid_checker = match &final_checker {
        Some(GateChecker::Exec { command, .. }) => !command.is_empty(),
        Some(_) => true,
        None => false,
    };
    if final_mode == GateMode::Auto && !has_valid_checker {
        return Err(anyhow!(
            "Automated gates must have a checker configured. Add --checker-command or use --mode manual"
        ));
    }

    Ok(GateDefinition {
        version: current.version,
        key: current.key,
        title: update.title.unwrap_or(current.title),
        description: update.description.unwrap_or(current.description),
        stage: update.stage.unwrap_or(current.stage),
        mode: final_mode,
        checker: final_checker,
        inputs: current.inputs,
        priority: update.priority.unwrap_or(current.priority),
        reserved: current.reserved,
        auto: final_mode == GateMode::Auto,
        example_integration: current.example_integration,
    })
}

fn derive_gate_registry_mutation(
    mut registry: crate::declarations::GateRegistry,
    request: &CapturedGateRegistryMutation,
) -> Result<DerivedGateRegistryMutation> {
    let (event, outcome) = match request {
        CapturedGateRegistryMutation::Define(definition) => {
            if registry.gates.contains_key(&definition.key) {
                return Err(
                    crate::storage::GateAlreadyExistsError::new(definition.key.as_str()).into(),
                );
            }
            registry
                .gates
                .insert(definition.key.clone(), definition.clone());
            (
                Event::draft_gate_definition_created(definition.key.clone()),
                CapturedGateRegistryOutcome::Defined,
            )
        }
        CapturedGateRegistryMutation::Update { key, update } => {
            let current = registry
                .gates
                .get(key)
                .cloned()
                .ok_or_else(|| crate::storage::GateNotFoundError::by_key(key))?;
            let updated = updated_gate_definition(current, update.clone())?;
            registry.gates.insert(key.clone(), updated.clone());
            (
                Event::draft_gate_definition_updated(key.clone()),
                CapturedGateRegistryOutcome::Updated(Box::new(updated)),
            )
        }
        CapturedGateRegistryMutation::Remove { key } => {
            if registry.gates.remove(key).is_none() {
                return Err(crate::storage::GateNotFoundError::by_key(key).into());
            }
            (
                Event::draft_gate_definition_removed(key.clone()),
                CapturedGateRegistryOutcome::Removed,
            )
        }
    };
    // Normalize through the declaration-owned serializer now so projection input
    // exactly matches what the authored file will contain (including null-valued
    // reserved-entry stripping).
    let bytes = crate::declarations::serialize_gate_registry(&registry)?;
    let registry = crate::declarations::parse_gate_registry(&bytes)?;
    Ok(DerivedGateRegistryMutation {
        registry,
        event,
        outcome,
    })
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Add a single gate to an issue.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn add_gate(&self, issue_id: &str, gate_key: String) -> Result<Vec<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.add_gates(issue_id, &[gate_key])
            .map(|(_, warnings)| warnings)
    }

    /// Add multiple gates to an issue atomically.
    ///
    /// Returns (result, warnings) where warnings contains lease warnings if any.
    pub fn add_gates(
        &self,
        issue_id: &str,
        gate_keys: &[String],
    ) -> Result<(GateAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Validate input
        if gate_keys.is_empty() {
            return Err(anyhow!("Must provide at least one gate key"));
        }

        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let CapturedIssueMutationOutcome::GatesAdded {
            added,
            already_exist,
        } = self.publish_captured_issue_mutation(CapturedIssueMutation::AddGates {
            issue_id: full_id,
            gate_keys: gate_keys.to_vec(),
        })?
        else {
            return Err(anyhow!("unexpected result from captured gate addition"));
        };

        Ok((
            GateAddResult {
                added,
                already_exist,
            },
            warnings,
        ))
    }

    /// Remove multiple gates from an issue.
    ///
    /// Returns (result, warnings) where warnings contains lease warnings if any.
    pub fn remove_gates(
        &self,
        issue_id: &str,
        gate_keys: &[String],
    ) -> Result<(GateRemoveResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Validate input
        if gate_keys.is_empty() {
            return Err(anyhow!("Must provide at least one gate key"));
        }

        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let CapturedIssueMutationOutcome::GatesRemoved { removed, not_found } = self
            .publish_captured_issue_mutation(CapturedIssueMutation::RemoveGates {
                issue_id: full_id,
                gate_keys: gate_keys.to_vec(),
            })?
        else {
            return Err(anyhow!("unexpected result from captured gate removal"));
        };

        Ok((GateRemoveResult { removed, not_found }, warnings))
    }

    /// Mark a gate as passed.
    ///
    /// For an automated gate this runs the checker; for a manual gate it records
    /// the attestation. Automated gates whose latest run passed at the current
    /// `HEAD` are skipped, and a gate declaring inputs a prior run's verdict
    /// already covers reuses that verdict
    /// ([`check_gate`](Self::check_gate)); `force` refuses both and executes.
    /// Manual attestations are always recorded as fresh evidence.
    ///
    /// Returns a [`GatePassOutcome`] carrying any warnings (e.g. lease warnings)
    /// and whether the checker was skipped.
    pub fn pass_gate(
        &self,
        issue_id: &str,
        gate_key: String,
        by: Option<String>,
        force: bool,
    ) -> Result<GatePassOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Validate the actor through the one `Assignee` path before it is stored
        // on the gate state.
        let by = by
            .map(|s| s.parse::<crate::domain::Assignee>())
            .transpose()?;
        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let issue = self.storage.load_issue(&full_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(GateNotRequiredError {
                issue_id: full_id,
                gate_key,
            }
            .into());
        }

        // Load the gate registry once, up front: its mode governs both the
        // short-circuit guard below and the auto/manual arm selected further down.
        let registry = self.storage.load_gate_registry()?;
        let gate = registry
            .gates
            .get(&gate_key)
            .ok_or_else(|| crate::storage::GateNotFoundError::single(&gate_key))?;

        // Skip the checker when the gate is CURRENTLY passed AND its latest run
        // passed at the current HEAD, unless --force was given. Requiring the
        // current `gates_status` to be Passed prevents a false success when the
        // gate was reset to Pending (e.g. removed and re-added) while a stale
        // passing run still lingers at this HEAD. A `None` HEAD (no git / no
        // commit) cannot prove the prior pass is still valid, so we fall through.
        //
        let recorded = issue.gates_status.get(&gate_key);
        let current_passed = matches!(recorded, Some(s) if s.status == GateStatus::Passed);
        if gate.mode == GateMode::Auto
            && !force
            && current_passed
            && self.gate_passed_at_head(&full_id, &gate_key)?
        {
            return Ok(GatePassOutcome {
                warnings,
                already_passed: true,
            });
        }

        // Check if gate is automated - if so, run the checker instead
        if gate.mode == GateMode::Auto {
            // Smart behavior: auto-run the checker. `force` is the explicit
            // re-run request, so it also refuses a verdict recorded by an
            // earlier run over the same declared inputs.
            let source = if force {
                crate::commands::VerdictSource::Execute
            } else {
                crate::commands::VerdictSource::ReuseUnchangedInputs
            };
            let result = self.check_gate_with(&full_id, &gate_key, source)?;
            if result.status != GateRunStatus::Passed {
                return Err(GatePassFailed {
                    issue_id: full_id,
                    gate_key,
                    status: result.status,
                    exit_code: result.exit_code,
                    result,
                    warnings,
                }
                .into());
            }
            if let Some(message) = result.message {
                warnings.push(message);
            }
            return Ok(GatePassOutcome {
                warnings,
                already_passed: false,
            });
        }

        // Manual gate: a pass must be attributable (jit:1d59070d REQ-03). A
        // manual gate has no checker to run, so evaluating it bare would
        // silently record a pass with no evidence of who attested — the same
        // "silent success" failure mode REQ-01/REQ-02 close for gate define.
        // Require --by before writing anything.
        let Some(by) = by else {
            return Err(ManualGateAttestationRequiredError {
                issue_id: full_id,
                gate_key,
            }
            .into());
        };

        self.publish_captured_issue_mutation(CapturedIssueMutation::SetManualGateStatus {
            issue_id: full_id.clone(),
            gate_key,
            status: GateStatus::Passed,
            by: Some(by),
        })?;

        // Check if Gated issue can now transition to Done
        self.auto_transition_to_done(&full_id)?;

        Ok(GatePassOutcome {
            warnings,
            already_passed: false,
        })
    }

    /// Pass every required gate for an issue in declaration order, fail-fast.
    ///
    /// Each gate is passed via [`pass_gate`](Self::pass_gate), so automated gates
    /// inherit skip-if-passed-at-`HEAD` and manual gates record fresh evidence.
    /// On the FIRST gate that does not pass,
    /// the underlying error is propagated unchanged and no later gate is
    /// attempted, so the caller can map it to the right exit code (checker
    /// failure, runner error, etc.). An issue with no required gates succeeds
    /// with an empty result set.
    pub fn pass_all_gates(
        &self,
        issue_id: &str,
        by: Option<String>,
        force: bool,
    ) -> Result<PassAllOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let issue = self.storage.load_issue(&full_id)?;

        // Fail-fast: pass each required gate in order, stopping at the first that
        // does not pass and propagating its error unchanged. `try_fold` short-
        // circuits on the first `Err`, so later gates are never attempted.
        let results = issue.gates_required.iter().try_fold(
            Vec::with_capacity(issue.gates_required.len()),
            |mut results, gate_key| {
                let outcome = self.pass_gate(&full_id, gate_key.clone(), by.clone(), force)?;
                results.push(GatePassAllEntry {
                    gate_key: gate_key.clone(),
                    already_passed: outcome.already_passed,
                    warnings: outcome.warnings,
                });
                Ok::<_, anyhow::Error>(results)
            },
        )?;

        Ok(PassAllOutcome { results })
    }

    /// Evaluate one gate for several issues in sequence.
    ///
    /// Each issue is passed through [`pass_gate`](Self::pass_gate), preserving
    /// its per-issue locking, verdict reuse, and run recording semantics. An
    /// error for one issue is retained in that issue's result and does not
    /// prevent later issue identifiers from being evaluated.
    pub fn pass_gate_many(
        &self,
        issue_ids: &[String],
        gate_key: &str,
        by: Option<String>,
        force: bool,
    ) -> GateEvaluateManyOutcome
    where
        S: crate::storage::RepositoryStateStore,
    {
        let results = issue_ids
            .iter()
            .map(|issue_id| GateEvaluateManyEntry {
                issue_id: issue_id.clone(),
                result: self.pass_gate(issue_id, gate_key.to_string(), by.clone(), force),
            })
            .collect();

        GateEvaluateManyOutcome { results }
    }

    /// Whether the gate's latest run passed at the current `HEAD` commit.
    ///
    /// This is the historical-run half of the skip predicate. The full skip in
    /// [`pass_gate`](Self::pass_gate) ALSO requires the issue's current
    /// `gates_status` for this gate to be `Passed`, so a gate reset to `Pending`
    /// (e.g. removed and re-added) is never skipped even if a stale passing run
    /// lingers at this HEAD.
    ///
    /// Compares the current `HEAD` (resolved from the same repo root the checker
    /// runs in, so the commit is apples-to-apples with the value stamped into
    /// [`GateRunResult::commit`](crate::domain::GateRunResult)) against the latest
    /// recorded run for `(issue, gate)`. Returns `true` only when that run passed
    /// AND both commits are present and equal. A missing `HEAD` (no git / no
    /// commit) yields `false`, since the prior pass cannot be proven current.
    fn gate_passed_at_head(&self, full_id: &str, gate_key: &str) -> Result<bool> {
        let head = crate::gate_execution::get_git_commit(&self.checker_repo_root()?);
        let Some(head) = head else {
            return Ok(false);
        };
        let passed = self
            .get_last_gate_run(full_id, gate_key)?
            .is_some_and(|run| {
                run.status == GateRunStatus::Passed && run.commit.as_deref() == Some(head.as_str())
            });
        Ok(passed)
    }

    /// Mark a gate as failed.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn fail_gate(
        &self,
        issue_id: &str,
        gate_key: String,
        by: Option<String>,
    ) -> Result<Vec<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Validate the actor through the one `Assignee` path before it is stored
        // on the gate state.
        let by = by
            .map(|s| s.parse::<crate::domain::Assignee>())
            .transpose()?;
        let full_id = self.storage.resolve_issue_id(issue_id)?;

        // Collect warnings instead of printing
        let mut warnings = Vec::new();
        if let Some(warning) = self.require_active_lease(&full_id)? {
            warnings.push(warning);
        }

        let issue = self.storage.load_issue(&full_id)?;

        if !issue.gates_required.contains(&gate_key) {
            return Err(anyhow!(
                "Gate '{}' is not required for this issue",
                gate_key
            ));
        }

        // Check if gate is automated - reject manual pass/fail
        let registry = self.storage.load_gate_registry()?;
        if let Some(gate) = registry.gates.get(&gate_key) {
            if gate.mode == GateMode::Auto {
                return Err(anyhow!(
                    "Gate '{}' is automated and cannot be manually failed. Use 'jit gate evaluate {} {}' to run the checker.",
                    gate_key, full_id, gate_key
                ));
            }
        }

        self.publish_captured_issue_mutation(CapturedIssueMutation::SetManualGateStatus {
            issue_id: full_id,
            gate_key,
            status: GateStatus::Failed,
            by,
        })?;

        Ok(warnings)
    }

    pub fn list_gates(&self) -> Result<Vec<GateDefinition>> {
        let registry = self.storage.load_gate_registry()?;
        Ok(registry.gates.into_values().collect())
    }

    fn publish_gate_registry_mutation(
        &self,
        request: CapturedGateRegistryMutation,
    ) -> Result<CapturedGateRegistryOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{
            assemble_config, finalize_gate_registry_edit, render_capture_closure, CaptureBudget,
            CaptureSpec, MutationIntent, RepositoryEntry, VirtualPath,
        };

        let layout = self.require_layout()?;
        let context = super::production_mutation_context();
        let budget = CaptureBudget {
            max_listings: 64,
            max_bytes: 256 * 1024 * 1024,
            max_depth: 16,
        };
        let registries = || -> Result<[VirtualPath; 5]> {
            Ok([
                VirtualPath::CONFIG,
                VirtualPath::INVARIANTS,
                VirtualPath::RULES,
                VirtualPath::GATES,
                VirtualPath::EVENTS,
            ])
        };

        with_mutation_session(
            &self.storage,
            &layout,
            "gate registry mutation",
            |session| {
                let Some(image_one) = capture_or_retry(
                    session.capture(CaptureSpec::phase_one(registries()?, budget)?),
                )?
                else {
                    return Ok(SessionStep::Retry);
                };
                let config = assemble_config(&image_one)?;
                let rules_text = super::image_repo_bytes(&image_one, ".jit/rules.toml")?
                    .map(String::from_utf8)
                    .transpose()?;
                let closure = render_capture_closure(
                    image_one.layout(),
                    &config,
                    &[],
                    rules_text.as_deref(),
                )?;
                let mut spec = CaptureSpec::phase_one(registries()?, budget)?;
                spec.discover_paths(closure)?;
                let Some(image) = capture_or_retry(session.capture(spec))? else {
                    return Ok(SessionStep::Retry);
                };
                let gates_path = VirtualPath::GATES;
                let registry = match image.entry(&gates_path)? {
                    RepositoryEntry::File { bytes, .. } => {
                        crate::declarations::parse_gate_registry(bytes)?
                    }
                    RepositoryEntry::Absent => crate::declarations::GateRegistry::default(),
                    _ => return Err(anyhow!("captured gate registry is not an ordinary file")),
                };
                let derived = derive_gate_registry_mutation(registry, &request)?;
                let mut declarations = crate::repository_state::declarations_from_image(&image)?;
                declarations.gates = derived.registry.clone();
                let intents = vec![
                    MutationIntent::EditGateRegistry {
                        registry: Box::new(derived.registry),
                    },
                    MutationIntent::RecordEvent {
                        phase: 1,
                        event: Box::new(derived.event),
                    },
                ];
                let plan = finalize_gate_registry_edit(
                    &layout,
                    &image,
                    &context,
                    &intents,
                    declarations.borrowed(),
                )?;
                Ok(SessionStep::Apply(plan, derived.outcome))
            },
        )
    }

    pub fn add_gate_definition(
        &self,
        key: String,
        title: String,
        description: String,
        auto: bool,
        example_integration: Option<String>,
        stage: crate::declarations::GateStage,
    ) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Global operation - enforce common history with main
        crate::commands::worktree::enforce_main_only_operations()?;

        self.publish_gate_registry_mutation(CapturedGateRegistryMutation::Define(
            GateDefinition {
                version: 1,
                key,
                title,
                description,
                stage,
                mode: if auto {
                    GateMode::Auto
                } else {
                    GateMode::Manual
                },
                checker: None,
                inputs: None,
                priority: 100,
                reserved: std::collections::HashMap::new(),
                auto,
                example_integration,
            },
        ))?;
        Ok(())
    }

    /// Define a new gate with full control over stage, mode, and checker.
    ///
    /// `example_integration` is an optional usage snippet stored on the gate
    /// definition (surfaced by `jit gate show`). Returns an error if `key` is
    /// already registered, or if `mode` is [`GateMode::Auto`](crate::declarations::GateMode)
    /// without a `checker`. A `gate_definition_created` event is appended
    /// (@/inv/event-log), mirroring [`update_gate`](Self::update_gate)'s
    /// `gate_definition_updated` event.
    #[allow(clippy::too_many_arguments)]
    pub fn define_gate(
        &self,
        key: String,
        title: String,
        description: String,
        stage: crate::declarations::GateStage,
        mode: crate::declarations::GateMode,
        checker: Option<crate::declarations::GateChecker>,
        priority: u32,
        example_integration: Option<String>,
    ) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Global operation - enforce common history with main
        crate::commands::worktree::enforce_main_only_operations()?;

        // Validate: auto gates must have checker
        if mode == crate::declarations::GateMode::Auto && checker.is_none() {
            return Err(anyhow!(
                "Automated gates must have a checker configured. Add --checker-command or use --mode manual"
            ));
        }

        // For manual gates, ignore any provided checker
        let final_checker = if mode == crate::declarations::GateMode::Manual {
            None
        } else {
            checker
        };

        self.publish_gate_registry_mutation(CapturedGateRegistryMutation::Define(
            GateDefinition {
                version: 1,
                key,
                title,
                description,
                stage,
                mode,
                checker: final_checker,
                inputs: None,
                priority,
                reserved: std::collections::HashMap::new(),
                auto: mode == crate::declarations::GateMode::Auto,
                example_integration,
            },
        ))?;
        Ok(())
    }

    /// Update an existing gate's mutable fields in the registry.
    ///
    /// Each set field of `update` replaces the gate's current value; every
    /// unprovided field keeps its current value; the clearable checker fields
    /// ([`FieldEdit::Clear`]) reset to none/empty. The gate KEY is its identity
    /// and is never modified. The authored registry, its
    /// `gate_definition_updated` audit event, and configured derived projections
    /// publish through one repository-state transaction; no per-issue
    /// `gates_status` is touched.
    ///
    /// Updating a key that is not in the registry is a typed
    /// [`GateNotFoundError`](crate::storage::GateNotFoundError) (exit code `3`).
    /// Mirroring [`define_gate`](Self::define_gate): a gate that ends up in
    /// automated mode must have a configured checker (and an exec checker must
    /// have a non-empty command), while a manual gate carries no checker.
    ///
    /// Returns the updated [`GateDefinition`] so callers can render it.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::{CommandExecutor, GateUpdate};
    /// use jit::declarations::{GateMode, GateStage};
    /// use jit::InMemoryStorage;
    ///
    /// std::env::set_var("JIT_TEST_MODE", "1"); // skip the main-history guard
    /// let storage = InMemoryStorage::new();
    /// storage.add_data_file("config.toml", "");
    /// let layout = storage.repository_layout();
    /// let executor = CommandExecutor::new(storage).with_layout(layout);
    /// executor
    ///     .define_gate(
    ///         "tests".to_string(),
    ///         "Old title".to_string(),
    ///         "Desc".to_string(),
    ///         GateStage::Postcheck,
    ///         GateMode::Manual,
    ///         None,
    ///         100,
    ///         None,
    ///     )
    ///     .unwrap();
    ///
    /// // Update only the title; every other field is preserved.
    /// let updated = executor
    ///     .update_gate(
    ///         "tests",
    ///         GateUpdate {
    ///             title: Some("All Tests Pass".to_string()),
    ///             ..Default::default()
    ///         },
    ///     )
    ///     .unwrap();
    /// assert_eq!(updated.title, "All Tests Pass");
    /// assert_eq!(updated.description, "Desc");
    /// ```
    pub fn update_gate(&self, key: &str, update: GateUpdate) -> Result<GateDefinition>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Global operation - enforce common history with main (mirrors define_gate).
        crate::commands::worktree::enforce_main_only_operations()?;
        match self.publish_gate_registry_mutation(CapturedGateRegistryMutation::Update {
            key: key.to_string(),
            update,
        })? {
            CapturedGateRegistryOutcome::Updated(updated) => Ok(*updated),
            _ => unreachable!("update request has one outcome"),
        }
    }

    pub fn remove_gate_definition(&self, key: &str) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Global operation - enforce common history with main
        crate::commands::worktree::enforce_main_only_operations()?;

        self.publish_gate_registry_mutation(CapturedGateRegistryMutation::Remove {
            key: key.to_string(),
        })?;
        Ok(())
    }

    pub fn show_gate_definition(&self, key: &str) -> Result<GateDefinition> {
        let registry = self.storage.load_gate_registry()?;
        registry
            .gates
            .get(key)
            .cloned()
            .ok_or_else(|| crate::storage::GateNotFoundError::by_key(key).into())
    }

    // Preset management methods

    pub fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>> {
        self.storage.list_gate_presets()
    }

    pub fn show_gate_preset(
        &self,
        name: &str,
    ) -> Result<crate::gate_presets::GatePresetDefinition> {
        self.storage.get_gate_preset(name)
    }

    /// Apply a gate preset to an issue.
    ///
    /// Returns (result, warnings) where warnings contains lease warnings if any.
    pub fn apply_gate_preset(
        &self,
        issue_id: &str,
        preset_name: &str,
        timeout_override: Option<u64>,
        skip_precheck: bool,
        skip_postcheck: bool,
        except_gates: &[String],
    ) -> Result<(GateAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, finalize_gate_registry_edit};

        let layout = self.require_layout()?;
        let context = super::production_mutation_context();
        with_mutation_attempts("gate preset application", || {
            let (expected_target, expected_mode) = {
                let mut preflight = self.storage.open_mutation_session(layout.clone())?;
                let Some(image) = capture_gate_preset_image(&mut *preflight, false, None)? else {
                    return Ok(AttemptOutcome::Retry);
                };
                let issues = super::captured_active_issues(&image)?;
                let target = super::resolve_issue_from_capture(&issues, issue_id)?;
                let config = crate::repository_state::assemble_config(&image)?;
                (
                    target,
                    self.config_manager.enforcement_mode_from_config(&config)?,
                )
            };
            let claims_guard = (expected_mode != crate::config::EnforcementMode::Off)
                .then(|| super::claims_mutation_guard(&layout))
                .transpose()?
                .flatten();
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(image) = capture_gate_preset_image(&mut *session, true, None)? else {
                return Ok(AttemptOutcome::Retry);
            };
            let issues = super::captured_active_issues(&image)?;
            let target = super::resolve_issue_from_capture(&issues, issue_id)?;
            let config = crate::repository_state::assemble_config(&image)?;
            let mode = self.config_manager.enforcement_mode_from_config(&config)?;
            if target != expected_target || mode != expected_mode {
                return Ok(AttemptOutcome::Retry);
            }
            let warnings = super::captured_lease_warnings(
                mode,
                std::slice::from_ref(&target),
                &issues,
                claims_guard.as_ref(),
            )?;
            let preset = parse_captured_gate_presets(&image)?
                .remove(preset_name)
                .ok_or_else(|| crate::storage::PresetNotFoundError::new(preset_name))?;
            let issue = captured_gate_preset_issue(&image, &target)?;
            let derived = derive_gate_preset_application(
                issue,
                captured_gate_registry(&image)?,
                &preset,
                timeout_override,
                skip_precheck,
                skip_postcheck,
                except_gates,
            )?;
            if derived.intents.is_empty() {
                return Ok(AttemptOutcome::Done((derived.result, warnings)));
            }
            let plan = if derived.edits_registry {
                let mut declarations = crate::repository_state::declarations_from_image(&image)?;
                declarations.gates = derived.registry;
                finalize_gate_registry_edit(
                    &layout,
                    &image,
                    &context,
                    &derived.intents,
                    declarations.borrowed(),
                )?
            } else {
                finalize(&layout, &image, &context, &derived.intents)?
            };
            classify_apply(session.apply(&plan), (derived.result, warnings))
        })
    }

    /// List all gate run results for an issue, optionally filtered by gate key.
    ///
    /// Results are sorted newest-first by `started_at`.
    pub fn list_gate_runs(
        &self,
        issue_id: &str,
        gate_key_filter: Option<&str>,
    ) -> Result<Vec<crate::domain::GateRunResult>> {
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let mut runs = self.storage.list_gate_runs_for_issue(&full_id)?;

        if let Some(key) = gate_key_filter {
            runs.retain(|r| r.gate_key == key);
        }

        runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
        Ok(runs)
    }

    /// Load a single gate run result by run ID.
    pub fn get_gate_run_result(&self, run_id: &str) -> Result<crate::domain::GateRunResult> {
        self.storage.load_gate_run_result(run_id)
    }

    pub fn create_gate_preset(
        &self,
        preset_name: &str,
        from_issue_id: &str,
    ) -> Result<std::path::PathBuf>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::gate_presets::{GatePresetDefinition, GateTemplate};

        crate::gate_presets::validate_preset_name(preset_name)
            .map_err(|error| crate::errors::InvalidArgumentError::new(error.to_string()))?;

        let layout = self.require_layout()?;
        let context = super::production_mutation_context();
        with_mutation_session(&self.storage, &layout, "gate preset creation", |session| {
            let Some(image) = capture_gate_preset_image(session, false, Some(preset_name))? else {
                return Ok(SessionStep::Retry);
            };
            let issues = super::captured_active_issues(&image)?;
            let full_id = super::resolve_issue_from_capture(&issues, from_issue_id)?;
            let presets_dir = crate::repository_state::VirtualPath::GATE_PRESETS;
            let target = crate::repository_state::VirtualPath::data(format!(
                "{}/{preset_name}.json",
                presets_dir.relative().as_str()
            ))?;
            if !matches!(
                image.entry(&target)?,
                crate::repository_state::RepositoryEntry::Absent
            ) {
                return Err(crate::errors::AlreadyExistsError::new(format!(
                    "Custom gate preset '{preset_name}' already exists"
                ))
                .into());
            }
            let issue = captured_gate_preset_issue(&image, &full_id)?;
            if issue.gates_required.is_empty() {
                return Err(anyhow!("Issue has no gates to create preset from"));
            }
            let registry = captured_gate_registry(&image)?;
            let gates = issue
                .gates_required
                .iter()
                .map(|gate_key| {
                    let gate = registry
                        .gates
                        .get(gate_key)
                        .ok_or_else(|| crate::storage::GateNotFoundError::in_registry(gate_key))?;
                    Ok(GateTemplate {
                        key: gate.key.clone(),
                        title: gate.title.clone(),
                        description: gate.description.clone(),
                        stage: gate.stage,
                        mode: gate.mode,
                        checker: gate.checker.clone(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let preset = GatePresetDefinition {
                name: preset_name.to_string(),
                description: format!("Custom preset created from issue {}", issue.short_id()),
                gates,
            };
            let intents = [crate::repository_state::MutationIntent::CreateGatePreset {
                preset: Box::new(preset),
            }];
            let plan = crate::repository_state::finalize(&layout, &image, &context, &intents)?;
            Ok(SessionStep::Apply(plan, layout.resolve(&target)?))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{seed_gate_registry, seed_issue, seed_repo_file};
    use crate::declarations::GateDefinition;
    use crate::declarations::{GateChecker, GateMode, GateStage};
    use crate::storage::InMemoryStorage;
    use std::collections::HashMap;

    fn setup() -> CommandExecutor<InMemoryStorage> {
        let storage = InMemoryStorage::new();

        // Create config with enforcement off for test backward compatibility
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        seed_repo_file(&storage, ".jit/config.toml", config_toml);

        crate::commands::test_helpers::memory_executor(storage)
    }

    #[test]
    fn test_pass_of_automated_gate_runs_checker() {
        let executor = setup();

        // Define an automated gate that will pass
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "auto-gate".to_string(),
            GateDefinition {
                version: 1,
                key: "auto-gate".to_string(),
                title: "Automated Gate".to_string(),
                description: "Auto gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 0".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with the gate
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .add_gate(&issue_id, "auto-gate".to_string())
            .unwrap();

        // Smart pass should auto-run the checker
        let result = executor.pass_gate(
            &issue_id,
            "auto-gate".to_string(),
            Some("human:test".to_string()),
            false,
        );

        assert!(
            result.is_ok(),
            "Pass of automated gate should run checker and succeed"
        );

        // Verify gate is marked as passed
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.gates_status.get("auto-gate").unwrap().status,
            crate::domain::GateStatus::Passed
        );
    }

    /// Register a simple manual gate so it can be added to issues in tests.
    fn define_manual_gate(executor: &CommandExecutor<InMemoryStorage>, key: &str) {
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            key.to_string(),
            GateDefinition {
                version: 1,
                key: key.to_string(),
                title: key.to_string(),
                description: String::new(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                inputs: None,
                priority: 100,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);
    }

    #[test]
    fn test_add_existing_gate_is_noop() {
        let executor = setup();
        define_manual_gate(&executor, "g1");

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);

        // Add the gate for real, then snapshot updated_at.
        let (res1, _) = executor.add_gates(&issue_id, &["g1".to_string()]).unwrap();
        assert_eq!(res1.added, vec!["g1".to_string()]);
        let updated_after_add = executor.storage.load_issue(&issue_id).unwrap().updated_at;

        // Re-adding the same gate is a no-op: it must not bump updated_at.
        let (res2, _) = executor.add_gates(&issue_id, &["g1".to_string()]).unwrap();
        assert!(res2.added.is_empty());
        assert_eq!(res2.already_exist, vec!["g1".to_string()]);
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().updated_at,
            updated_after_add,
            "re-adding an existing gate must not bump updated_at"
        );
    }

    #[test]
    fn test_remove_absent_gate_is_noop() {
        let executor = setup();
        define_manual_gate(&executor, "g1");

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        let updated_before = executor.storage.load_issue(&issue_id).unwrap().updated_at;

        // Removing a gate the issue does not have is a no-op.
        let (res, _) = executor
            .remove_gates(&issue_id, &["g1".to_string()])
            .unwrap();
        assert!(res.removed.is_empty());
        assert_eq!(res.not_found, vec!["g1".to_string()]);
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().updated_at,
            updated_before,
            "removing an absent gate must not bump updated_at"
        );
    }

    #[test]
    fn test_pass_of_automated_gate_that_fails() {
        let executor = setup();

        // Define an automated gate that will fail
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "auto-gate".to_string(),
            GateDefinition {
                version: 1,
                key: "auto-gate".to_string(),
                title: "Automated Gate".to_string(),
                description: "Auto gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "exit 1".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 100,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with the gate
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .add_gate(&issue_id, "auto-gate".to_string())
            .unwrap();

        // Smart pass runs the checker, which fails
        let result = executor.pass_gate(
            &issue_id,
            "auto-gate".to_string(),
            Some("human:test".to_string()),
            false,
        );

        assert!(
            result.is_err(),
            "Pass should fail when the automated checker fails"
        );

        // Verify gate is marked as failed (checker failed)
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.gates_status.get("auto-gate").unwrap().status,
            crate::domain::GateStatus::Failed
        );
    }

    #[test]
    fn test_manual_pass_of_manual_gate_should_succeed() {
        let executor = setup();

        // Define a manual gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "manual-gate".to_string(),
            GateDefinition {
                version: 1,
                key: "manual-gate".to_string(),
                title: "Manual Gate".to_string(),
                description: "Manual gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                inputs: None,
                priority: 100,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with the gate
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .add_gate(&issue_id, "manual-gate".to_string())
            .unwrap();

        // Manual pass of manual gate should succeed
        let result = executor.pass_gate(
            &issue_id,
            "manual-gate".to_string(),
            Some("human:reviewer".to_string()),
            false,
        );
        assert!(result.is_ok(), "Manual pass of manual gate should succeed");

        // Verify gate is marked as passed
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.gates_status.get("manual-gate").unwrap().status,
            crate::domain::GateStatus::Passed
        );
    }

    /// REQ-03 (jit:1d59070d): a bare pass_gate on a manual gate — no `by` —
    /// must not silently record a pass. A manual gate has no checker to run,
    /// so evaluating it without an attestor is indistinguishable from an
    /// unverified pass.
    #[test]
    fn test_manual_pass_of_manual_gate_without_by_fails() {
        let executor = setup();
        define_manual_gate(&executor, "manual-gate");

        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .add_gate(&issue_id, "manual-gate".to_string())
            .unwrap();

        let result = executor.pass_gate(&issue_id, "manual-gate".to_string(), None, false);
        let err = result.expect_err("bare pass_gate on a manual gate must fail");
        assert!(
            err.downcast_ref::<ManualGateAttestationRequiredError>()
                .is_some(),
            "expected ManualGateAttestationRequiredError, got: {err}"
        );
        assert!(
            err.to_string().contains("--by <attestor>"),
            "error must hint the attested form: {err}"
        );

        // Nothing was written after the rejected pass: canonical gate addition
        // left the gate Pending.
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue.gates_status["manual-gate"].status,
            GateStatus::Pending,
            "a rejected bare pass must leave the gate Pending"
        );
    }

    #[derive(Default)]
    struct OneFailure(std::sync::Mutex<Option<crate::storage::TransactionFailurePoint>>);

    impl crate::storage::TransactionFailureInjector for OneFailure {
        fn check(&self, point: &crate::storage::TransactionFailurePoint) -> std::io::Result<()> {
            let mut selected = self.0.lock().unwrap();
            if selected.as_ref() == Some(point) {
                selected.take();
                return Err(std::io::Error::other("injected gate registry failure"));
            }
            Ok(())
        }
    }

    #[test]
    fn test_gate_definition_apply_failure_recovers_registry_and_event_together() {
        use crate::storage::RepositoryStateStore;

        let failures = std::sync::Arc::new(OneFailure(std::sync::Mutex::new(Some(
            crate::storage::TransactionFailurePoint::RepositoryAfterAction { action: 0 },
        ))));
        let storage = InMemoryStorage::with_repository_state_failures(failures);
        seed_repo_file(&storage, ".jit/config.toml", "");
        let recovered = storage.without_repository_state_failures();
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage).with_layout(layout.clone());

        assert!(executor
            .define_gate(
                "atomic-review".to_string(),
                "Atomic review".to_string(),
                "Must publish with its event".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
                100,
                None,
            )
            .is_err());

        // Opening the next clean session performs recovery of any prepared
        // aggregate residue before the read assertions.
        drop(
            recovered
                .open_mutation_session(layout)
                .expect("recovery converges"),
        );
        assert!(recovered.load_gate_registry().unwrap().gates.is_empty());
        assert!(recovered.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_gate_preset_apply_failure_recovers_registry_issue_and_events_together() {
        use crate::storage::RepositoryStateStore;

        let failures = std::sync::Arc::new(OneFailure(std::sync::Mutex::new(Some(
            crate::storage::TransactionFailurePoint::RepositoryAfterAction { action: 0 },
        ))));
        let storage = InMemoryStorage::with_repository_state_failures(failures);
        seed_repo_file(
            &storage,
            ".jit/config.toml",
            "[worktree]\nenforce_leases = \"off\"\n",
        );
        let issue = crate::domain::types::fixture_issue("Preset target".into(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(&storage, issue);
        let recovered = storage.without_repository_state_failures();
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage).with_layout(layout.clone());

        assert!(executor
            .apply_gate_preset(&issue_id, "plan-review", None, false, false, &[])
            .is_err());
        drop(
            recovered
                .open_mutation_session(layout)
                .expect("recovery converges"),
        );
        assert!(recovered.load_gate_registry().unwrap().gates.is_empty());
        assert!(recovered
            .load_issue(&issue_id)
            .unwrap()
            .gates_required
            .is_empty());
        assert!(recovered.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_gate_preset_create_revalidates_absent_target_against_concurrent_creator() {
        use crate::gate_presets::{GatePresetDefinition, GateTemplate};
        use crate::repository_state::{finalize, MutationContext, MutationIntent};
        use crate::storage::{RepositoryStateStore, RepositoryStateStoreError};

        let executor = setup();
        define_manual_gate(&executor, "review");
        let mut issue = crate::domain::types::fixture_issue("Preset source".into(), String::new());
        issue.gates_required.push("review".into());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        let layout = executor.storage.repository_layout();

        let mut stale_session = executor
            .storage
            .open_mutation_session(layout.clone())
            .unwrap();
        let stale_image =
            capture_gate_preset_image(stale_session.as_mut(), false, Some("race-preset"))
                .unwrap()
                .unwrap();
        let preset = GatePresetDefinition {
            name: "race-preset".into(),
            description: "Captured race".into(),
            gates: vec![GateTemplate {
                key: "review".into(),
                title: "review".into(),
                description: String::new(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
            }],
        };
        let stale_plan = finalize(
            &layout,
            &stale_image,
            &MutationContext::deterministic(
                [91; 32],
                chrono::DateTime::parse_from_rfc3339("2026-07-22T12:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            ),
            &[MutationIntent::CreateGatePreset {
                preset: Box::new(preset),
            }],
        )
        .unwrap();

        executor
            .create_gate_preset("race-preset", &issue_id)
            .unwrap();
        assert!(matches!(
            stale_session.apply(&stale_plan),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
        assert!(executor
            .storage
            .read_repo_file(".jit/config/gate-presets/race-preset.json")
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_memory_gate_preset_create_is_visible_to_list_and_show() {
        let executor = setup();
        define_manual_gate(&executor, "review");
        let mut issue = crate::domain::types::fixture_issue("Preset source".into(), String::new());
        issue.gates_required.push("review".into());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);

        let path = executor
            .create_gate_preset("team-review", &issue_id)
            .unwrap();
        assert_eq!(
            path,
            executor
                .storage
                .repository_layout()
                .data_root()
                .join("config/gate-presets/team-review.json")
        );

        let listed = executor
            .list_gate_presets()
            .unwrap()
            .into_iter()
            .find(|preset| preset.name == "team-review")
            .expect("created custom preset is listed");
        assert_eq!(listed.gate_count, 1);
        let shown = executor.show_gate_preset("team-review").unwrap();
        assert_eq!(shown.gates.len(), 1);
        assert_eq!(shown.gates[0].key, "review");
    }

    #[test]
    fn test_memory_gate_preset_reader_rejects_non_regular_json_occupant() {
        let executor = setup();
        let path = crate::repository_state::VirtualPath::data("config/gate-presets/directory.json")
            .unwrap();
        executor.storage.repository_state().entries.insert(
            path,
            crate::repository_state::RepositoryEntry::Directory {
                identity: crate::repository_state::EntryIdentity::for_bytes(
                    "memory-preset-directory",
                    b"directory",
                )
                .unwrap(),
                mode: crate::repository_state::FileMode::Executable,
            },
        );

        let error = executor
            .list_gate_presets()
            .expect_err("a .json directory must be rejected");
        assert!(error.to_string().contains("must be an ordinary file"));
    }

    #[test]
    fn test_gate_preset_create_reports_occupied_malformed_target_as_already_exists() {
        let executor = setup();
        define_manual_gate(&executor, "review");
        let mut issue = crate::domain::types::fixture_issue("Preset source".into(), String::new());
        issue.gates_required.push("review".into());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        seed_repo_file(
            executor.storage(),
            ".jit/config/gate-presets/occupied.json",
            "{ malformed",
        );

        let error = executor
            .create_gate_preset("occupied", &issue_id)
            .expect_err("an occupied exact target must not be parsed or overwritten");
        assert!(error
            .downcast_ref::<crate::errors::AlreadyExistsError>()
            .is_some());
    }

    #[test]
    fn test_gate_definition_updates_configured_projection_in_same_mutation() {
        let storage = InMemoryStorage::new();
        seed_repo_file(
            &storage,
            ".jit/config.toml",
            r#"
[item_kinds.gate]
section = "success_criteria"
id-pattern = "[a-z][a-z0-9-]*"
markers = []
link-namespaces = []
scope = "project"
source = { toml = ".jit/gates.toml", table = "gates", id-field = "key", text-field = "description" }
source-of-truth = "registry-first"

[item_kinds.rule]
section = "success_criteria"
id-pattern = "[a-z][a-z0-9-]*"
markers = []
link-namespaces = []
scope = "project"
source = { toml = ".jit/rules.toml", table = "rules", id-field = "name", text-field = "description" }
source-of-truth = "registry-first"

[projection.gates]
kind = ["rule", "gate"]
mode = "separate-file"
target = "gates.md"
style = "full"
"#,
        );
        seed_repo_file(&storage, ".jit/rules.toml", "");
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage).with_layout(layout);

        executor
            .define_gate(
                "projected-review".to_string(),
                "Projected review".to_string(),
                "Rendered from the registry".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
                100,
                None,
            )
            .unwrap();

        let projection = executor
            .storage
            .read_repo_file("gates.md")
            .unwrap()
            .expect("configured projection is materialized");
        assert!(projection.contains("projected-review"));
        let event = executor.storage.read_events().unwrap().pop().unwrap();
        match event {
            Event::GateDefinitionCreated { id, timestamp, .. } => {
                assert!(!id.is_empty());
                assert_ne!(timestamp, chrono::DateTime::UNIX_EPOCH);
            }
            other => panic!("unexpected registry event: {other:?}"),
        }
    }

    #[test]
    fn test_gate_registry_retry_rederives_over_concurrent_definition() {
        use crate::repository_state::{
            finalize_gate_registry_edit, CaptureBudget, CaptureSpec, MutationContext,
            MutationIntent, VirtualPath,
        };
        use crate::storage::{RepositoryStateStore, RepositoryStateStoreError};

        let storage = InMemoryStorage::new();
        seed_repo_file(&storage, ".jit/config.toml", "");
        let layout = storage.repository_layout();
        let spec = || {
            CaptureSpec::phase_one(
                [
                    VirtualPath::data("config.toml").unwrap(),
                    VirtualPath::data("invariants.toml").unwrap(),
                    VirtualPath::data("rules.toml").unwrap(),
                    VirtualPath::data("gates.toml").unwrap(),
                    VirtualPath::data("events.jsonl").unwrap(),
                ],
                CaptureBudget {
                    max_listings: 0,
                    max_bytes: 1024 * 1024,
                    max_depth: 4,
                },
            )
            .unwrap()
        };
        let request = CapturedGateRegistryMutation::Define(GateDefinition {
            version: 1,
            key: "ours".to_string(),
            title: "Ours".to_string(),
            description: String::new(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: HashMap::new(),
            auto: false,
            example_integration: None,
            inputs: None,
        });
        let context = MutationContext::deterministic(
            [72; 32],
            chrono::DateTime::parse_from_rfc3339("2026-07-20T12:34:56Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );
        let derive_plan = |image: &crate::repository_state::RepositoryImage| {
            let gates = match image
                .entry(&VirtualPath::data("gates.toml").unwrap())
                .unwrap()
            {
                crate::repository_state::RepositoryEntry::File { bytes, .. } => {
                    crate::declarations::parse_gate_registry(bytes).unwrap()
                }
                crate::repository_state::RepositoryEntry::Absent => Default::default(),
                other => panic!("unexpected gate registry entry: {other:?}"),
            };
            let derived = derive_gate_registry_mutation(gates, &request).unwrap();
            let mut declarations = crate::repository_state::declarations_from_image(image).unwrap();
            declarations.gates = derived.registry.clone();
            let intents = vec![
                MutationIntent::EditGateRegistry {
                    registry: Box::new(derived.registry),
                },
                MutationIntent::RecordEvent {
                    phase: 1,
                    event: Box::new(derived.event),
                },
            ];
            finalize_gate_registry_edit(&layout, image, &context, &intents, declarations.borrowed())
                .unwrap()
        };
        let event_bytes = |plan: &crate::repository_state::MaterializationPlan| {
            plan.delta()
                .actions()
                .iter()
                .find_map(|action| match action {
                    crate::repository_state::RepositoryAction::WriteFile {
                        path, bytes, ..
                    } if path == &VirtualPath::data("events.jsonl").unwrap() => Some(bytes.clone()),
                    _ => None,
                })
                .unwrap()
        };

        let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
        let first_image = first_session.capture(spec()).unwrap();
        let first_plan = derive_plan(&first_image);

        let mut concurrent = crate::declarations::GateRegistry::default();
        concurrent.gates.insert(
            "theirs".to_string(),
            GateDefinition {
                version: 1,
                key: "theirs".to_string(),
                title: "Theirs".to_string(),
                description: String::new(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                inputs: None,
                priority: 100,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
            },
        );
        seed_gate_registry(&storage, &concurrent);
        assert!(matches!(
            first_session.apply(&first_plan),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
        drop(first_session);

        let mut retry_session = storage.open_mutation_session(layout.clone()).unwrap();
        let retry_image = retry_session.capture(spec()).unwrap();
        let retry_plan = derive_plan(&retry_image);
        assert_eq!(event_bytes(&first_plan), event_bytes(&retry_plan));
        retry_session.apply(&retry_plan).unwrap();

        let registry = storage.load_gate_registry().unwrap();
        assert!(registry.gates.contains_key("ours"));
        assert!(registry.gates.contains_key("theirs"));
        let event = storage.read_events().unwrap().pop().unwrap();
        match event {
            Event::GateDefinitionCreated { id, timestamp, .. } => {
                assert!(!id.is_empty());
                assert_ne!(timestamp, chrono::DateTime::UNIX_EPOCH);
            }
            other => panic!("unexpected registry event: {other:?}"),
        }
    }

    #[test]
    fn test_gate_registry_missing_update_and_remove_keep_typed_errors() {
        for request in [
            CapturedGateRegistryMutation::Update {
                key: "missing-update".to_string(),
                update: GateUpdate {
                    title: Some("No gate".to_string()),
                    ..Default::default()
                },
            },
            CapturedGateRegistryMutation::Remove {
                key: "missing-remove".to_string(),
            },
        ] {
            let error = match derive_gate_registry_mutation(Default::default(), &request) {
                Ok(_) => panic!("missing gate unexpectedly accepted"),
                Err(error) => error,
            };
            assert!(error
                .downcast_ref::<crate::storage::GateNotFoundError>()
                .is_some());
        }
    }
}
