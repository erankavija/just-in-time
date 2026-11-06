//! Quality gate operations

use super::*;
use crate::declarations::GateMode;
use crate::domain::identity::GateKey;
use crate::domain::{GateRunResult, GateRunStatus};
use crate::gate_presets::GatePresetCatalog;

/// Error returned when `jit gate evaluate` runs an automated checker that does not pass.
///
/// The checker result is preserved so CLI callers can report the failed status
/// without disagreeing with the persisted `gates_status` value.
#[derive(Debug, thiserror::Error)]
#[error(
    "Gate '{gate_key}' failed for issue {issue_id}. Checker status: {status}, exit code: {exit_code:?}. Inspect details with: jit gate status {issue_id} {gate_key} (add --all for run history, --findings for structured findings)"
)]
pub struct GatePassFailed {
    /// Issue whose gate was checked.
    pub issue_id: IssueId,
    /// Gate key that failed.
    pub gate_key: GateKey,
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
    pub issue_id: IssueId,
    /// Gate key that is not in the issue's required set.
    pub gate_key: GateKey,
}

/// Error returned by `jit gate status` when its two positionals look swapped:
/// the first names a registered gate and the second resolves to an issue.
///
/// An invalid-argument condition (exit code `2`) whose message names the
/// canonical argument order.
#[derive(Debug)]
pub struct GateStatusArgumentsTransposedError {
    /// The issue, as written in the gate position.
    pub issue: String,
    /// The gate key, as written in the issue position.
    pub gate_key: String,
}

impl GateStatusArgumentsTransposedError {
    /// The invocation with the arguments in canonical order.
    pub fn canonical_command(&self) -> String {
        format!("jit gate status {} {}", self.issue, self.gate_key)
    }
}

impl std::fmt::Display for GateStatusArgumentsTransposedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "'{}' is a gate key and '{}' is an issue; the issue id and gate key look transposed.\n  Did you mean: {}",
            self.gate_key,
            self.issue,
            self.canonical_command()
        )
    }
}

impl std::error::Error for GateStatusArgumentsTransposedError {}

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
    pub issue_id: IssueId,
    /// Manual gate key that was targeted without `--by`.
    pub gate_key: GateKey,
}

/// Outcome of a successful [`pass_gate`](GateOps::pass_gate) call.
///
/// Carries any warnings gathered along the way (e.g. lease warnings).
#[derive(Debug, Clone)]
pub struct GatePassOutcome {
    /// Warnings gathered while passing the gate (e.g. lease warnings).
    pub warnings: Vec<String>,
}

/// Per-gate entry in a [`PassAllOutcome`].
///
/// Records which gate was passed, plus any warnings gathered.
#[derive(Debug, Clone)]
pub struct GatePassAllEntry {
    /// The gate that was passed.
    pub gate_key: GateKey,
    /// Warnings gathered while passing this gate.
    pub warnings: Vec<String>,
}

/// Outcome of a successful [`pass_all_gates`](GateOps::pass_all_gates) run.
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
    pub issue_id: IssueId,
    /// The per-issue gate result, or the error that prevented a verdict.
    pub result: Result<GatePassOutcome>,
}

/// Per-issue results from [`GateOps::pass_gate_many`].
pub struct GateEvaluateManyOutcome {
    /// Results in the same order as the requested issue identifiers.
    pub results: Vec<GateEvaluateManyEntry>,
}

/// Result of adding multiple gates
#[derive(Debug, Serialize)]
pub struct GateAddResult {
    pub added: Vec<GateKey>,
    pub already_exist: Vec<GateKey>,
}

/// Result of removing multiple gates
#[derive(Debug, Serialize)]
pub struct GateRemoveResult {
    pub removed: Vec<GateKey>,
    pub not_found: Vec<GateKey>,
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

/// Mutable fields for [`update_gate`](GateOps::update_gate).
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
    Update { key: GateKey, update: GateUpdate },
    Remove { key: GateKey },
}

enum CapturedGateRegistryOutcome {
    Defined,
    Updated(Box<GateDefinition>),
    Removed,
}

struct DerivedGateRegistryMutation {
    registry: crate::declarations::GateRegistry,
    event: EventBody,
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
        max_bytes: super::CAPTURE_MAX_BYTES,
        max_depth: 16,
    };

fn captured_gate_preset_fixed_paths() -> Vec<crate::repository_state::VirtualPath> {
    use crate::repository_state::VirtualPath;
    vec![
        VirtualPath::CONFIG,
        VirtualPath::INDEX,
        VirtualPath::INVARIANTS,
        VirtualPath::RULES,
        VirtualPath::GATES,
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
    let mut spec = CaptureSpec::new(
        captured_gate_preset_fixed_paths(),
        GATE_PRESET_CAPTURE_BUDGET,
    )?;
    spec.discover_tail(VirtualPath::EVENTS)?;
    spec.discover_listings([presets_dir.clone(), issues_dir.clone()])?;
    let create_path = create_target
        .map(|name| VirtualPath::data(format!("{}/{name}.json", presets_dir.relative().as_str())))
        .transpose()?;
    let Some(image) = super::capture_discovering(session, spec, |image, spec| {
        let issue_paths = crate::repository_state::captured_issue_ids(image)?
            .iter()
            .map(VirtualPath::issue_record)
            .collect::<Vec<_>>();
        let closure = if include_projection_closure {
            let config = assemble_config(image)?;
            let rules_text =
                super::image_virtual_bytes(image, &crate::repository_state::VirtualPath::RULES)?
                    .map(String::from_utf8)
                    .transpose()?;
            render_capture_closure(image.layout(), &config, &[], rules_text.as_deref())?
        } else {
            Vec::new()
        };
        Ok(spec.discover_paths(
            GatePresetCatalog::paths(image)?
                .into_iter()
                .chain(issue_paths)
                .chain(closure)
                .chain(create_path.clone()),
        )?)
    })?
    else {
        return Ok(None);
    };
    Ok(Some(image))
}

fn captured_gate_registry(
    image: &crate::repository_state::RepositoryImage,
) -> Result<crate::declarations::GateRegistry> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};
    match image.entry(&VirtualPath::GATES)? {
        RepositoryEntry::File { content, .. } => {
            crate::declarations::parse_gate_registry(content.bytes()).map_err(Into::into)
        }
        RepositoryEntry::Absent => Ok(crate::declarations::GateRegistry::default()),
        _ => Err(anyhow!("captured gate registry is not an ordinary file")),
    }
}

fn captured_gate_preset_issue(
    image: &crate::repository_state::RepositoryImage,
    issue_id: &IssueId,
) -> Result<Issue> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};
    let path = VirtualPath::issue_record(issue_id);
    let issue = match image.entry(&path)? {
        RepositoryEntry::File { content, .. } => {
            crate::repository_state::decode_issue_record(&path, content.bytes())?
        }
        RepositoryEntry::Absent => {
            return Err(crate::storage::IssueNotFoundError::new(issue_id.clone()).into())
        }
        _ => return Err(anyhow!("captured issue path is not an ordinary file")),
    };
    if &issue.id != issue_id {
        return Err(anyhow!(
            "captured issue identity mismatch: requested {issue_id}, found {}",
            issue.id
        ));
    }
    Ok(issue)
}

/// Which of a preset's gates an application leaves out.
struct PresetGateExclusions<'a> {
    skip_precheck: bool,
    skip_postcheck: bool,
    except_gates: &'a [GateKey],
}

fn derive_gate_preset_application(
    issue: Issue,
    mut registry: crate::declarations::GateRegistry,
    preset: &crate::gate_presets::GatePresetDefinition,
    timeout_override: Option<u64>,
    PresetGateExclusions {
        skip_precheck,
        skip_postcheck,
        except_gates,
    }: PresetGateExclusions<'_>,
    context: &crate::repository_state::MutationContext,
) -> Result<DerivedGatePresetApplication> {
    use crate::declarations::{GateChecker, GateStage};
    use crate::repository_state::MutationIntent;

    let gates = preset
        .gates
        .iter()
        .filter(|gate| {
            !(skip_precheck && gate.stage == GateStage::Precheck
                || skip_postcheck && gate.stage == GateStage::Postcheck
                || except_gates.iter().any(|key| key == &gate.key))
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
                    EventBody::draft_gate_definition_updated(gate.key)
                } else {
                    EventBody::draft_gate_definition_created(gate.key)
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
        context,
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
    })
}

fn derive_gate_registry_mutation(
    mut registry: crate::declarations::GateRegistry,
    request: &CapturedGateRegistryMutation,
) -> Result<DerivedGateRegistryMutation> {
    let (event, outcome) = match request {
        CapturedGateRegistryMutation::Define(definition) => {
            if registry.gates.contains_key(&definition.key) {
                return Err(crate::storage::GateAlreadyExistsError::new(&definition.key).into());
            }
            registry
                .gates
                .insert(definition.key.clone(), definition.clone());
            (
                EventBody::draft_gate_definition_created(definition.key.clone()),
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
            registry.gates.insert(updated.key.clone(), updated.clone());
            (
                EventBody::draft_gate_definition_updated(updated.key.clone()),
                CapturedGateRegistryOutcome::Updated(Box::new(updated)),
            )
        }
        CapturedGateRegistryMutation::Remove { key } => {
            let removed = registry
                .gates
                .remove(key)
                .ok_or_else(|| crate::storage::GateNotFoundError::by_key(key))?;
            (
                EventBody::draft_gate_definition_removed(removed.key),
                CapturedGateRegistryOutcome::Removed,
            )
        }
    };
    crate::declarations::normalize_gate_registry_for_document(&mut registry);
    Ok(DerivedGateRegistryMutation {
        registry,
        event,
        outcome,
    })
}

/// Quality-gate operations of a [`CommandExecutor`].
pub struct GateOps<'a, S: IssueStore> {
    pub(super) exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The quality-gate family.
    pub fn gates(&self) -> GateOps<'_, S> {
        GateOps { exec: self }
    }
}

impl<S: IssueStore> GateOps<'_, S> {
    /// Add a single gate to an issue.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn add_gate(&self, issue_id: &IssueId, gate_key: &GateKey) -> Result<Vec<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.add_gates(issue_id, std::slice::from_ref(gate_key))
            .map(|(_, warnings)| warnings)
    }

    /// Add multiple gates to an issue atomically.
    ///
    /// Returns (result, warnings) where warnings contains lease warnings if any.
    pub fn add_gates(
        &self,
        issue_id: &IssueId,
        gate_keys: &[GateKey],
    ) -> Result<(GateAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Validate input
        if gate_keys.is_empty() {
            return Err(anyhow!("Must provide at least one gate key"));
        }

        let (_, outcome, warnings) =
            self.exec
                .publish_captured_issue_mutation(CapturedIssueMutation::AddGates {
                    issue_id: issue_id.clone(),
                    gate_keys: gate_keys.to_vec(),
                })?;
        let CapturedIssueMutationOutcome::GatesAdded {
            added,
            already_exist,
        } = outcome
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
        issue_id: &IssueId,
        gate_keys: &[GateKey],
    ) -> Result<(GateRemoveResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Validate input
        if gate_keys.is_empty() {
            return Err(anyhow!("Must provide at least one gate key"));
        }

        let (_, outcome, warnings) =
            self.exec
                .publish_captured_issue_mutation(CapturedIssueMutation::RemoveGates {
                    issue_id: issue_id.clone(),
                    gate_keys: gate_keys.to_vec(),
                })?;
        let CapturedIssueMutationOutcome::GatesRemoved { removed, not_found } = outcome else {
            return Err(anyhow!("unexpected result from captured gate removal"));
        };

        Ok((GateRemoveResult { removed, not_found }, warnings))
    }

    /// Mark a gate as passed, then complete the issue if that cleared the last
    /// unpassed gate of a gated issue.
    ///
    /// Returns a [`GatePassOutcome`] carrying any warnings (e.g. lease warnings).
    pub fn pass_gate(
        &self,
        issue_id: &IssueId,
        gate_key: &GateKey,
        by: Option<String>,
        force: bool,
    ) -> Result<GatePassOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let outcome = self.record_gate_pass(issue_id, gate_key, by, force)?;
        self.exec.auto_transition_to_done(issue_id)?;
        Ok(outcome)
    }

    /// Record a gate's pass without advancing the issue.
    ///
    /// For an automated gate this runs the checker; for a manual gate it records
    /// the attestation. A gate declaring inputs a prior run's verdict already
    /// covers reuses that verdict ([`check_gate`](GateOps::check_gate)); `force`
    /// refuses reuse and executes. Manual attestations are always recorded as
    /// fresh evidence.
    fn record_gate_pass(
        &self,
        issue_id: &IssueId,
        gate_key: &GateKey,
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
        let full_id = issue_id.clone();
        let issue = self.exec.storage.load_issue(&full_id)?;

        if !issue.gates_required.contains(gate_key) {
            return Err(GateNotRequiredError {
                issue_id: full_id,
                gate_key: gate_key.clone(),
            }
            .into());
        }

        let registry = self.exec.storage.load_gate_registry()?;
        let gate = registry
            .gates
            .get(gate_key)
            .ok_or_else(|| crate::storage::GateNotFoundError::single(gate_key))?;

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
            let (result, mut warnings) = self.check_gate_with(&full_id, gate_key, source)?;
            if result.status != GateRunStatus::Passed {
                return Err(GatePassFailed {
                    issue_id: full_id,
                    gate_key: gate_key.clone(),
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
            return Ok(GatePassOutcome { warnings });
        }

        // Manual gate: a pass must be attributable (jit:1d59070d REQ-03). A
        // manual gate has no checker to run, so evaluating it bare would
        // silently record a pass with no evidence of who attested — the same
        // "silent success" failure mode REQ-01/REQ-02 close for gate define.
        // Require --by before writing anything.
        let Some(by) = by else {
            return Err(ManualGateAttestationRequiredError {
                issue_id: full_id,
                gate_key: gate_key.clone(),
            }
            .into());
        };

        let (_, _, warnings) = self.exec.publish_captured_issue_mutation(
            CapturedIssueMutation::SetManualGateStatus {
                issue_id: full_id.clone(),
                gate_key: gate_key.clone(),
                status: GateStatus::Passed,
                by: Some(by),
            },
        )?;

        Ok(GatePassOutcome { warnings })
    }

    /// Pass every required gate for an issue in declaration order, fail-fast.
    ///
    /// Each gate is recorded via `record_gate_pass`, so
    /// automated gates inherit its verdict reuse and manual gates record fresh
    /// evidence. The issue advances only after the last gate, so a re-evaluated
    /// gate that now fails cannot follow a completion. On the FIRST gate that does
    /// not pass, the underlying error is propagated unchanged and no later gate is
    /// attempted, so the caller can map it to the right exit code (checker
    /// failure, runner error, etc.). An issue with no required gates succeeds
    /// with an empty result set.
    pub fn pass_all_gates(
        &self,
        issue_id: &IssueId,
        by: Option<String>,
        force: bool,
    ) -> Result<PassAllOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let full_id = issue_id.clone();
        let issue = self.exec.storage.load_issue(&full_id)?;

        // Fail-fast: pass each required gate in order, stopping at the first that
        // does not pass and propagating its error unchanged. `try_fold` short-
        // circuits on the first `Err`, so later gates are never attempted.
        let results = issue.gates_required.iter().try_fold(
            Vec::with_capacity(issue.gates_required.len()),
            |mut results, gate_key| {
                let outcome = self.record_gate_pass(&full_id, gate_key, by.clone(), force)?;
                results.push(GatePassAllEntry {
                    gate_key: gate_key.clone(),
                    warnings: outcome.warnings,
                });
                Ok::<_, anyhow::Error>(results)
            },
        )?;
        self.exec.auto_transition_to_done(&full_id)?;

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
        issue_ids: &[IssueId],
        gate_key: &GateKey,
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
                result: self.pass_gate(issue_id, gate_key, by.clone(), force),
            })
            .collect();

        GateEvaluateManyOutcome { results }
    }

    /// Mark a gate as failed.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn fail_gate(
        &self,
        issue_id: &IssueId,
        gate_key: &GateKey,
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
        let full_id = issue_id.clone();
        let issue = self.exec.storage.load_issue(&full_id)?;

        if !issue.gates_required.contains(gate_key) {
            return Err(GateNotRequiredError {
                issue_id: full_id,
                gate_key: gate_key.clone(),
            }
            .into());
        }

        // Check if gate is automated - reject manual pass/fail
        let registry = self.exec.storage.load_gate_registry()?;
        if let Some(gate) = registry.gates.get(gate_key) {
            if gate.mode == GateMode::Auto {
                return Err(anyhow!(
                    "Gate '{}' is automated and cannot be manually failed. Use 'jit gate evaluate {} {}' to run the checker.",
                    gate_key, full_id, gate_key
                ));
            }
        }

        let (_, _, warnings) = self.exec.publish_captured_issue_mutation(
            CapturedIssueMutation::SetManualGateStatus {
                issue_id: full_id,
                gate_key: gate_key.clone(),
                status: GateStatus::Failed,
                by,
            },
        )?;

        Ok(warnings)
    }

    pub fn list_gates(&self) -> Result<Vec<GateDefinition>> {
        let registry = self.exec.storage.load_gate_registry()?;
        Ok(registry.gates.into_values().collect())
    }

    /// Whether `jit gate status <issue> <gate_key>` has its positionals swapped:
    /// `issue` names a registered gate and `gate_key` resolves to an issue.
    pub fn are_gate_status_arguments_transposed(&self, issue: &str, gate_key: &str) -> bool {
        self.list_gates()
            .is_ok_and(|gates| gates.iter().any(|gate| gate.key.as_str() == issue))
            && gate_key
                .parse::<crate::domain::identity::IssueIdPrefix>()
                .is_ok_and(|prefix| self.exec.storage.resolve_issue_id(&prefix).is_ok())
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

        let layout = self.exec.require_layout()?;
        let context = super::production_mutation_context();
        let budget = CaptureBudget {
            max_listings: 64,
            max_bytes: 256 * 1024 * 1024,
            max_depth: 16,
        };
        let registries = || -> Result<[VirtualPath; 4]> {
            Ok([
                VirtualPath::CONFIG,
                VirtualPath::INVARIANTS,
                VirtualPath::RULES,
                VirtualPath::GATES,
            ])
        };

        with_mutation_session(
            &self.exec.storage,
            &layout,
            "gate registry mutation",
            |session| {
                let mut spec = CaptureSpec::new(registries()?, budget)?;
                spec.discover_tail(VirtualPath::EVENTS)?;
                let Some(image) = super::capture_discovering(session, spec, |image, spec| {
                    let config = assemble_config(image)?;
                    let rules_text = super::image_virtual_bytes(
                        image,
                        &crate::repository_state::VirtualPath::RULES,
                    )?
                    .map(String::from_utf8)
                    .transpose()?;
                    Ok(spec.discover_paths(render_capture_closure(
                        image.layout(),
                        &config,
                        &[],
                        rules_text.as_deref(),
                    )?)?)
                })?
                else {
                    return Ok(SessionStep::Retry);
                };
                let gates_path = VirtualPath::GATES;
                let registry = match image.entry(&gates_path)? {
                    RepositoryEntry::File { content, .. } => {
                        crate::declarations::parse_gate_registry(content.bytes())?
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

    /// Define a new gate with full control over stage, mode, and checker.
    ///
    /// Returns an error if `key` is already registered,
    /// or if `mode` is [`GateMode::Auto`](crate::declarations::GateMode) without a
    /// `checker`. A
    /// `gate_definition_created` event is appended (@/inv/event-log), mirroring
    /// [`update_gate`](Self::update_gate)'s `gate_definition_updated` event.
    #[allow(clippy::too_many_arguments)]
    pub fn define_gate(
        &self,
        key: &GateKey,
        title: String,
        description: String,
        stage: crate::declarations::GateStage,
        mode: crate::declarations::GateMode,
        checker: Option<crate::declarations::GateChecker>,
        priority: u32,
    ) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Global operation - enforce common history with main
        self.exec.require_common_history_with_main()?;

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
                key: key.clone(),
                title,
                description,
                stage,
                mode,
                checker: final_checker,
                inputs: None,
                priority,
                reserved: std::collections::HashMap::new(),
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
    /// publish through one redo record; no per-issue
    /// `gates_status` is touched.
    ///
    /// Updating a key that is not in the registry is a typed
    /// [`GateNotFoundError`](crate::storage::GateNotFoundError) (exit code `3`).
    /// Mirroring [`define_gate`](Self::define_gate): a gate that ends up in
    /// automated mode must have a configured checker (and an exec checker must
    /// have a non-empty command), while a manual gate carries no checker.
    ///
    /// Refuses when another agent leases an issue that requires the gate.
    /// Returns the updated [`GateDefinition`] and the `warn`-mode lease warnings.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::commands::{CommandExecutor, GateUpdate};
    /// use jit::declarations::{GateMode, GateStage};
    /// use jit::InMemoryStorage;
    ///
    /// let storage = InMemoryStorage::new();
    /// storage.add_data_file("config.toml", "");
    /// let layout = storage.repository_layout();
    /// let executor = CommandExecutor::new(storage).with_layout(layout).unwrap();
    /// executor
    ///     .gates().define_gate(
    ///         &"tests".parse().unwrap(),
    ///         "Old title".to_string(),
    ///         "Desc".to_string(),
    ///         GateStage::Postcheck,
    ///         GateMode::Manual,
    ///         None,
    ///         100,
    ///     )
    ///     .unwrap();
    ///
    /// // Update only the title; every other field is preserved.
    /// let (updated, _warnings) = executor
    ///     .gates().update_gate(
    ///         &"tests".parse().unwrap(),
    ///         GateUpdate {
    ///             title: Some("All Tests Pass".to_string()),
    ///             ..Default::default()
    ///         },
    ///     )
    ///     .unwrap();
    /// assert_eq!(updated.title, "All Tests Pass");
    /// assert_eq!(updated.description, "Desc");
    /// ```
    pub fn update_gate(
        &self,
        key: &GateKey,
        update: GateUpdate,
    ) -> Result<(GateDefinition, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Global operation - enforce common history with main (mirrors define_gate).
        self.exec.require_common_history_with_main()?;
        let warnings = self.refuse_foreign_leases_on_gate_users(key)?;
        match self.publish_gate_registry_mutation(CapturedGateRegistryMutation::Update {
            key: key.clone(),
            update,
        })? {
            CapturedGateRegistryOutcome::Updated(updated) => Ok((*updated, warnings)),
            _ => unreachable!("update request has one outcome"),
        }
    }

    /// [`CommandExecutor::refuse_foreign_leases`] over the issues requiring `key`.
    fn refuse_foreign_leases_on_gate_users(&self, key: &GateKey) -> Result<Vec<String>> {
        let requiring = self
            .exec
            .storage
            .list_issues()?
            .into_iter()
            .filter(|issue| issue.gates_required.contains(key))
            .map(|issue| issue.id)
            .collect::<Vec<_>>();
        Ok(self
            .exec
            .refuse_foreign_leases(&requiring)?
            .into_iter()
            .map(|(_, warning)| warning)
            .collect())
    }

    /// Remove a gate definition, refusing when another agent leases an issue that
    /// requires it. Answers warnings in `warn` mode.
    pub fn remove_gate_definition(&self, key: &GateKey) -> Result<Vec<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        // Global operation - enforce common history with main
        self.exec.require_common_history_with_main()?;

        let warnings = self.refuse_foreign_leases_on_gate_users(key)?;
        self.publish_gate_registry_mutation(CapturedGateRegistryMutation::Remove {
            key: key.clone(),
        })?;
        Ok(warnings)
    }

    pub fn show_gate_definition(&self, key: &GateKey) -> Result<GateDefinition> {
        let registry = self.exec.storage.load_gate_registry()?;
        registry
            .gates
            .get(key)
            .cloned()
            .ok_or_else(|| crate::storage::GateNotFoundError::by_key(key).into())
    }

    // Preset management methods

    pub fn list_gate_presets(&self) -> Result<Vec<crate::gate_presets::PresetInfo>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        Ok(self.read_gate_preset_catalog()?.list())
    }

    pub fn show_gate_preset(&self, name: &str) -> Result<crate::gate_presets::GatePresetDefinition>
    where
        S: crate::storage::RepositoryStateStore,
    {
        Ok(self.read_gate_preset_catalog()?.get(name)?.clone())
    }

    fn read_gate_preset_catalog(&self) -> Result<GatePresetCatalog>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{CaptureSpec, VirtualPath};

        let layout = self.exec.require_layout()?;
        with_read_session(&self.exec.storage, &layout, "gate preset read", |session| {
            let mut spec =
                CaptureSpec::new([VirtualPath::GATE_PRESETS], GATE_PRESET_CAPTURE_BUDGET)?;
            spec.discover_listings([VirtualPath::GATE_PRESETS])?;
            let Some(image) = super::capture_discovering(session, spec, |image, spec| {
                Ok(spec.discover_paths(GatePresetCatalog::paths(image)?)?)
            })?
            else {
                return Ok(None);
            };
            Ok(Some(GatePresetCatalog::from_image(&image)?))
        })
    }

    /// Apply a gate preset to an issue.
    ///
    /// Returns (result, warnings) where warnings contains lease warnings if any.
    pub fn apply_gate_preset(
        &self,
        issue_id: &IssueId,
        preset_name: &str,
        timeout_override: Option<u64>,
        skip_precheck: bool,
        skip_postcheck: bool,
        except_gates: &[GateKey],
    ) -> Result<(GateAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, finalize_gate_registry_edit};

        let issue_id = &issue_id.clone();
        let layout = self.exec.require_layout()?;
        let context = super::production_mutation_context();
        with_mutation_attempts("gate preset application", || {
            let (expected_target, expected_mode) = {
                let mut preflight = self.exec.storage.open_mutation_session(layout.clone())?;
                let Some(preflight) =
                    self.exec
                        .capture_mutation_preflight(&mut *preflight, issue_id, |_| false)?
                else {
                    return Ok(AttemptOutcome::Retry);
                };
                let config = crate::repository_state::assemble_config(&preflight.image)?;
                (preflight.target.id, config.enforcement_mode())
            };
            let claims_guard = (expected_mode != crate::config::EnforcementMode::Off)
                .then(|| super::claims_mutation_guard(self.exec.require_worktree_paths()?))
                .transpose()?
                .flatten();
            let mut session = self.exec.storage.open_mutation_session(layout.clone())?;
            let Some(image) = capture_gate_preset_image(&mut *session, true, None)? else {
                return Ok(AttemptOutcome::Retry);
            };
            let issues = super::captured_active_issues(&image)?;
            let target = super::require_issue_in_capture(&issues, issue_id)?;
            let config = crate::repository_state::assemble_config(&image)?;
            let mode = config.enforcement_mode();
            if target != expected_target || mode != expected_mode {
                return Ok(AttemptOutcome::Retry);
            }
            let warnings = super::refuse_foreign_leases_held(
                mode,
                std::slice::from_ref(&target),
                claims_guard.as_ref(),
            )?;
            let catalog = GatePresetCatalog::from_image(&image)?;
            let preset = catalog.get(preset_name)?;
            let issue = captured_gate_preset_issue(&image, &target)?;
            let derived = derive_gate_preset_application(
                issue,
                captured_gate_registry(&image)?,
                preset,
                timeout_override,
                PresetGateExclusions {
                    skip_precheck,
                    skip_postcheck,
                    except_gates,
                },
                &context,
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
                finalize(&layout, &image, &context, &derived.intents, None)?
            };
            classify_apply(session.apply(&plan), (derived.result, warnings))
        })
    }

    /// List all gate run results for an issue, optionally filtered by gate key.
    ///
    /// Results are sorted newest-first by `started_at`.
    pub fn list_gate_runs(
        &self,
        issue_id: &IssueId,
        gate_key_filter: Option<&GateKey>,
    ) -> Result<Vec<crate::domain::GateRunResult>> {
        let full_id = issue_id.clone();
        let mut runs = self.exec.storage.list_gate_runs_for_issue(&full_id)?;

        if let Some(key) = gate_key_filter {
            runs.retain(|r| &r.gate_key == key);
        }

        runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
        Ok(runs)
    }

    /// Load a single gate run result belonging to the issue.
    pub fn get_gate_run_result(
        &self,
        issue_id: &IssueId,
        run_id: &crate::domain::identity::GateRunId,
    ) -> Result<crate::domain::GateRunResult> {
        self.exec.storage.load_gate_run_result(issue_id, run_id)
    }

    pub fn create_gate_preset(
        &self,
        preset_name: &str,
        from_issue_id: &IssueId,
    ) -> Result<std::path::PathBuf>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::gate_presets::{GatePresetDefinition, GateTemplate};

        crate::gate_presets::validate_preset_name(preset_name)
            .map_err(|error| crate::errors::InvalidArgumentError::new(error.to_string()))?;
        let from_issue_id = &from_issue_id.clone();

        let layout = self.exec.require_layout()?;
        let context = super::production_mutation_context();
        with_mutation_session(
            &self.exec.storage,
            &layout,
            "gate preset creation",
            |session| {
                let Some(image) = capture_gate_preset_image(session, false, Some(preset_name))?
                else {
                    return Ok(SessionStep::Retry);
                };
                let issues = super::captured_active_issues(&image)?;
                let full_id = super::require_issue_in_capture(&issues, from_issue_id)?;
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
                        let gate = registry.gates.get(gate_key).ok_or_else(|| {
                            crate::storage::GateNotFoundError::in_registry(gate_key)
                        })?;
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
                let plan =
                    crate::repository_state::finalize(&layout, &image, &context, &intents, None)?;
                Ok(SessionStep::Apply(plan, layout.resolve(&target)?))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{seed_gate_registry, seed_issue, seed_repo_file, setup};
    use crate::declarations::GateDefinition;
    use crate::declarations::{GateChecker, GateMode, GateStage};
    use crate::domain::EventBody;
    use crate::storage::InMemoryStorage;
    use std::collections::HashMap;

    #[test]
    fn test_gate_registry_guard_maps_supplied_checkout_comparisons() {
        use crate::test_cli::JitCli;

        for scenario in ["unresolved", "unrelated", "matching", "diverged"] {
            let cli = JitCli::uninitialised().unwrap();
            cli.init_git().unwrap().commit("base").unwrap();
            let base = match scenario {
                "unresolved" => None,
                "unrelated" => Some(
                    cli.git(["commit-tree", "HEAD^{tree}", "-m", "unrelated"])
                        .unwrap(),
                ),
                "matching" => Some(cli.git(["rev-parse", "HEAD"]).unwrap()),
                "diverged" => Some(
                    cli.git(["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "ahead"])
                        .unwrap(),
                ),
                _ => unreachable!(),
            };
            if let Some(base) = base {
                cli.git(["update-ref", "refs/remotes/origin/main", base.trim()])
                    .unwrap();
            }
            let executor =
                setup().with_worktree_paths(crate::test_utils::primary_checkout_paths(cli.root()));
            let key = "guarded".parse().unwrap();
            let result = executor.gates().define_gate(
                &key,
                "Guarded".to_string(),
                String::new(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
                100,
            );
            if scenario == "diverged" {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "Global operations require common history with main\n\n\
                     Your branch has diverged from origin/main. To proceed:\n  \
                     git fetch origin\n  \
                     git rebase origin/main"
                );
                assert!(!executor
                    .storage
                    .load_gate_registry()
                    .unwrap()
                    .gates
                    .contains_key(&key));
            } else {
                result.unwrap_or_else(|error| panic!("{scenario}: {error}"));
                assert!(executor
                    .storage
                    .load_gate_registry()
                    .unwrap()
                    .gates
                    .contains_key(&key));
            }
        }
    }

    #[test]
    fn test_pass_of_automated_gate_runs_checker() {
        let executor = setup();

        // Define an automated gate that will pass
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "auto-gate".parse().unwrap(),
            GateDefinition {
                version: 1,
                key: "auto-gate".parse().unwrap(),
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with the gate
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"auto-gate".parse().unwrap())
            .unwrap();

        // Smart pass should auto-run the checker
        let result = executor.gates().pass_gate(
            &issue_id,
            &"auto-gate".parse().unwrap(),
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
            issue
                .gates_status
                .get(&"auto-gate".parse().unwrap())
                .unwrap()
                .status,
            crate::domain::GateStatus::Passed
        );
    }

    #[test]
    fn test_are_gate_status_arguments_transposed_requires_a_gate_then_an_issue() {
        let executor = setup();
        define_manual_gate(&executor, &"tests".parse().unwrap());
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let id = issue.id.to_string();
        seed_issue(executor.storage(), issue);
        let gates = executor.gates();

        assert!(gates.are_gate_status_arguments_transposed("tests", &id));
        assert!(!gates.are_gate_status_arguments_transposed(&id, "tests"));
        assert!(!gates.are_gate_status_arguments_transposed("tests", "other"));
        assert!(!gates.are_gate_status_arguments_transposed("unknown", &id));
    }

    /// Register a simple manual gate so it can be added to issues in tests.
    fn define_manual_gate(executor: &CommandExecutor<InMemoryStorage>, key: &GateKey) {
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            key.clone(),
            GateDefinition {
                version: 1,
                key: key.clone(),
                title: key.to_string(),
                description: String::new(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                inputs: None,
                priority: 100,
                reserved: HashMap::new(),
            },
        );
        seed_gate_registry(executor.storage(), &registry);
    }

    #[test]
    fn test_add_existing_gate_is_noop() {
        let executor = setup();
        define_manual_gate(&executor, &"g1".parse().unwrap());

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);

        // Add the gate for real, then snapshot updated_at.
        let (res1, _) = executor
            .gates()
            .add_gates(&issue_id, &["g1".parse().unwrap()])
            .unwrap();
        assert_eq!(res1.added, vec!["g1".parse::<GateKey>().unwrap()]);
        let updated_after_add = executor.storage.load_issue(&issue_id).unwrap().updated_at;

        // Re-adding the same gate is a no-op: it must not bump updated_at.
        let (res2, _) = executor
            .gates()
            .add_gates(&issue_id, &["g1".parse().unwrap()])
            .unwrap();
        assert!(res2.added.is_empty());
        assert_eq!(res2.already_exist, vec!["g1".parse::<GateKey>().unwrap()]);
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().updated_at,
            updated_after_add,
            "re-adding an existing gate must not bump updated_at"
        );
    }

    #[test]
    fn test_remove_absent_gate_is_noop() {
        let executor = setup();
        define_manual_gate(&executor, &"g1".parse().unwrap());

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        let updated_before = executor.storage.load_issue(&issue_id).unwrap().updated_at;

        // Removing a gate the issue does not have is a no-op.
        let (res, _) = executor
            .gates()
            .remove_gates(&issue_id, &["g1".parse().unwrap()])
            .unwrap();
        assert!(res.removed.is_empty());
        assert_eq!(res.not_found, vec!["g1".parse::<GateKey>().unwrap()]);
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
            "auto-gate".parse().unwrap(),
            GateDefinition {
                version: 1,
                key: "auto-gate".parse().unwrap(),
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
                inputs: None,
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with the gate
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"auto-gate".parse().unwrap())
            .unwrap();

        // Smart pass runs the checker, which fails
        let result = executor.gates().pass_gate(
            &issue_id,
            &"auto-gate".parse().unwrap(),
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
            issue
                .gates_status
                .get(&"auto-gate".parse().unwrap())
                .unwrap()
                .status,
            crate::domain::GateStatus::Failed
        );
    }

    /// Pass the only manual gate of a `Gated` issue, optionally blocking a
    /// `Backlog` dependent; returns the session opens the pass took, the
    /// issue's state, and the dependent's state.
    fn pass_last_gate_of_gated_issue(with_dependent: bool) -> (usize, State, Option<State>) {
        use crate::commands::test_helpers::{memory_executor, SessionOpenCounter};

        let counter = SessionOpenCounter::new();
        let executor = memory_executor(
            InMemoryStorage::new().with_mutation_interruption_view(counter.clone()),
        );
        let key: GateKey = "review".parse().unwrap();
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            key.clone(),
            GateDefinition {
                version: 1,
                key: key.clone(),
                title: "Review".to_string(),
                description: String::new(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                inputs: None,
                priority: 100,
                reserved: HashMap::new(),
            },
        );
        seed_gate_registry(executor.storage(), &registry);
        let mut gated = crate::test_utils::fixture_issue("Gated".into(), String::new());
        gated.state = State::Gated;
        gated.gates_required = vec![key.clone()];
        let gated_id = gated.id.clone();
        seed_issue(executor.storage(), gated);
        let dependent_id = with_dependent.then(|| {
            let mut dependent = crate::test_utils::fixture_issue("Dependent".into(), String::new());
            dependent.dependencies = vec![gated_id.clone()];
            let id = dependent.id.clone();
            seed_issue(executor.storage(), dependent);
            id
        });
        let opened = counter.count();

        executor
            .gates()
            .pass_gate(&gated_id, &key, Some("human:reviewer".to_string()), false)
            .unwrap();

        (
            counter.count() - opened,
            executor.storage.load_issue(&gated_id).unwrap().state,
            dependent_id.map(|id| executor.storage.load_issue(&id).unwrap().state),
        )
    }

    #[test]
    fn test_pass_gate_completing_an_issue_promotes_its_dependent_in_the_same_publication() {
        let (baseline, _, _) = pass_last_gate_of_gated_issue(false);
        let (sessions, state, dependent) = pass_last_gate_of_gated_issue(true);

        assert_eq!(state, State::Done);
        assert_eq!(dependent, Some(State::Ready));
        assert_eq!(
            sessions, baseline,
            "the dependent's promotion must add no session to the automatic completion"
        );
    }

    #[test]
    fn test_manual_pass_of_manual_gate_should_succeed() {
        let executor = setup();

        // Define a manual gate
        let mut registry = executor.storage.load_gate_registry().unwrap();
        registry.gates.insert(
            "manual-gate".parse().unwrap(),
            GateDefinition {
                version: 1,
                key: "manual-gate".parse().unwrap(),
                title: "Manual Gate".to_string(),
                description: "Manual gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                inputs: None,
                priority: 100,
                reserved: HashMap::new(),
            },
        );
        seed_gate_registry(executor.storage(), &registry);

        // Create issue with the gate
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"manual-gate".parse().unwrap())
            .unwrap();

        // Manual pass of manual gate should succeed
        let result = executor.gates().pass_gate(
            &issue_id,
            &"manual-gate".parse().unwrap(),
            Some("human:reviewer".to_string()),
            false,
        );
        assert!(result.is_ok(), "Manual pass of manual gate should succeed");

        // Verify gate is marked as passed
        let issue = executor.storage.load_issue(&issue_id).unwrap();
        assert_eq!(
            issue
                .gates_status
                .get(&"manual-gate".parse().unwrap())
                .unwrap()
                .status,
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
        define_manual_gate(&executor, &"manual-gate".parse().unwrap());

        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"manual-gate".parse().unwrap())
            .unwrap();

        let result =
            executor
                .gates()
                .pass_gate(&issue_id, &"manual-gate".parse().unwrap(), None, false);
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
            issue.gates_status[&"manual-gate".parse().unwrap()].status,
            GateStatus::Pending,
            "a rejected bare pass must leave the gate Pending"
        );
    }

    /// Interrupt the mutation session once, at the selected moment, then let
    /// every later moment through. Shared by every gate-command durability
    /// test in this module, whatever repository state the command publishes.
    #[derive(Default)]
    struct OneFailure(std::sync::Mutex<Option<crate::storage::MutationPoint>>);

    impl crate::storage::MutationInterruptionInjector for OneFailure {
        fn check(&self, point: &crate::storage::MutationPoint) -> std::io::Result<()> {
            let mut selected = self.0.lock().unwrap();
            if selected.as_ref() == Some(point) {
                selected.take();
                return Err(std::io::Error::other("injected persistence failure"));
            }
            Ok(())
        }
    }

    #[test]
    fn test_gate_definition_apply_failure_recovers_registry_and_event_together() {
        use crate::storage::RepositoryStateStore;

        let failures = std::sync::Arc::new(OneFailure(std::sync::Mutex::new(Some(
            crate::storage::MutationPoint::BeforeCommitCheck,
        ))));
        let storage = InMemoryStorage::with_mutation_interruptions(failures);
        seed_repo_file(&storage, ".jit/config.toml", "");
        let recovered = storage.without_mutation_interruptions();
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage)
            .with_layout(layout.clone())
            .unwrap();

        assert!(executor
            .gates()
            .define_gate(
                &"atomic-review".parse().unwrap(),
                "Atomic review".to_string(),
                "Must publish with its event".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
                100,
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
            crate::storage::MutationPoint::BeforeCommitCheck,
        ))));
        let storage = InMemoryStorage::with_mutation_interruptions(failures);
        seed_repo_file(
            &storage,
            ".jit/config.toml",
            "[worktree]\nenforce_leases = \"off\"\n",
        );
        let issue = crate::test_utils::fixture_issue("Preset target".into(), String::new());
        let issue_id = issue.id.clone();
        seed_issue(&storage, issue);
        let recovered = storage.without_mutation_interruptions();
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage)
            .with_layout(layout.clone())
            .unwrap();

        assert!(executor
            .gates()
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

    /// REQ-05 (jit:7ca9a0dd): the manual attestation branch of gate evaluation
    /// publishes the attested gate status and its event as one plan through the
    /// same capture/apply path the automated branch uses, so a persistence
    /// failure reaches the caller as an error rather than a reported pass.
    #[test]
    fn test_pass_gate_manual_attestation_surfaces_persistence_failure() {
        use crate::storage::RepositoryStateStore;

        let executor = setup();
        define_manual_gate(&executor, &"manual-gate".parse().unwrap());
        let issue = crate::test_utils::fixture_issue("Test".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        executor
            .gates()
            .add_gate(&issue_id, &"manual-gate".parse().unwrap())
            .unwrap();

        // The attestation's own publication is the first this view sees: the
        // fixture above was seeded through the clean storage.
        let failures = std::sync::Arc::new(OneFailure(std::sync::Mutex::new(Some(
            crate::storage::MutationPoint::BeforeCommitCheck,
        ))));
        let injected = executor.storage().with_mutation_interruption_view(failures);
        let recovered = injected.without_mutation_interruptions();
        let layout = injected.repository_layout();
        let attesting = crate::commands::test_helpers::memory_executor(injected);

        let error = attesting
            .gates()
            .pass_gate(
                &issue_id,
                &"manual-gate".parse().unwrap(),
                Some("human:reviewer".to_string()),
                false,
            )
            .expect_err("a manual attestation whose persistence fails must not report success");
        assert!(
            format!("{error:#}").contains("injected persistence failure"),
            "the injected failure must be what surfaced: {error:#}"
        );

        drop(
            recovered
                .open_mutation_session(layout)
                .expect("recovery converges"),
        );
        assert_eq!(
            recovered.load_issue(&issue_id).unwrap().gates_status[&"manual-gate".parse().unwrap()]
                .status,
            GateStatus::Pending,
            "a failed attestation must leave no recorded pass"
        );
        assert!(
            !recovered
                .read_events()
                .unwrap()
                .iter()
                .any(|event| matches!(
                    event.body(),
                    EventBody::GatePassed { gate_key, .. } if gate_key == &"manual-gate".parse::<GateKey>().unwrap()
                )),
            "a failed attestation must log no gate-passed event"
        );
    }

    #[test]
    fn test_gate_preset_create_revalidates_absent_target_against_concurrent_creator() {
        use crate::gate_presets::{GatePresetDefinition, GateTemplate};
        use crate::repository_state::{finalize, MutationContext, MutationIntent};
        use crate::storage::{RepositoryStateStore, RepositoryStateStoreError};

        let executor = setup();
        define_manual_gate(&executor, &"review".parse().unwrap());
        let mut issue = crate::test_utils::fixture_issue("Preset source".into(), String::new());
        issue.gates_required.push("review".parse().unwrap());
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
                key: "review".parse().unwrap(),
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
            None,
        )
        .unwrap();

        executor
            .gates()
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
        define_manual_gate(&executor, &"review".parse().unwrap());
        let mut issue = crate::test_utils::fixture_issue("Preset source".into(), String::new());
        issue.gates_required.push("review".parse().unwrap());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);

        let path = executor
            .gates()
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
            .gates()
            .list_gate_presets()
            .unwrap()
            .into_iter()
            .find(|preset| preset.name == "team-review")
            .expect("created custom preset is listed");
        assert_eq!(listed.gate_count, 1);
        let shown = executor.gates().show_gate_preset("team-review").unwrap();
        assert_eq!(shown.gates.len(), 1);
        assert_eq!(shown.gates[0].key, "review".parse().unwrap());
    }

    #[test]
    fn test_gate_preset_create_reports_occupied_malformed_target_as_already_exists() {
        let executor = setup();
        define_manual_gate(&executor, &"review".parse().unwrap());
        let mut issue = crate::test_utils::fixture_issue("Preset source".into(), String::new());
        issue.gates_required.push("review".parse().unwrap());
        let issue_id = issue.id.clone();
        seed_issue(executor.storage(), issue);
        seed_repo_file(
            executor.storage(),
            ".jit/config/gate-presets/occupied.json",
            "{ malformed",
        );

        let error = executor
            .gates()
            .create_gate_preset("occupied", &issue_id)
            .expect_err("an occupied exact target must not be parsed or overwritten");
        assert!(error
            .downcast_ref::<crate::errors::AlreadyExistsError>()
            .is_some());
    }

    #[test]
    fn test_gate_definition_updates_configured_projection_in_same_mutation() {
        let storage = InMemoryStorage::new();
        let config = format!(
            "{}\n[projection.gates]\nkind = [\"rule\", \"gate\"]\nmode = \"separate-file\"\n\
             target = \"gates.md\"\nstyle = \"full\"\n",
            crate::test_utils::shipped_item_kind_tables(&["gate", "rule"])
        );
        seed_repo_file(&storage, ".jit/config.toml", &config);
        seed_repo_file(&storage, ".jit/rules.toml", "");
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage).with_layout(layout).unwrap();

        executor
            .gates()
            .define_gate(
                &"projected-review".parse().unwrap(),
                "Projected review".to_string(),
                "Rendered from the registry".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
                100,
            )
            .unwrap();

        let projection = executor
            .storage
            .read_repo_file("gates.md")
            .unwrap()
            .expect("configured projection is materialized");
        assert!(projection.contains("projected-review"));
        let event = executor.storage.read_events().unwrap().pop().unwrap();
        match event.body() {
            EventBody::GateDefinitionCreated { .. } => {
                assert!(!event.id().is_empty());
                assert_ne!(event.timestamp(), chrono::DateTime::UNIX_EPOCH);
            }
            other => panic!("unexpected registry event: {other:?}"),
        }
    }

    #[test]
    fn test_gate_registry_edits_preserve_authored_sections_and_order() {
        let authored = "# Input footprint: only files under the workspace are read.\n\n\
            [[gates]]\n\
            key = \"zulu\"\n\
            title = \"Zulu\"\n\
            description = \"Workspace check\"\n\
            stage = \"postcheck\"\n\
            mode = \"manual\"\n\
            \n\
            # Input footprint: the alpha checker reads declared schemas.\n\
            [[gates]]\n\
            key = \"alpha\"\n\
            title = \"Alpha\"\n\
            description = \"Schema check\"\n\
            stage = \"precheck\"\n\
            mode = \"manual\"\n\
            \n\
            # Input footprint: the beta checker is issue-local.\n\
            [[gates]]\n\
            key = \"beta\"\n\
            title = \"Beta\"\n\
            description = \"Issue check\"\n\
            stage = \"postcheck\"\n\
            mode = \"manual\"\n";
        let executor = setup();
        seed_repo_file(executor.storage(), ".jit/gates.toml", authored);

        executor
            .gates()
            .define_gate(
                &"new-gate".parse().unwrap(),
                "New gate".to_string(),
                "New check".to_string(),
                GateStage::Postcheck,
                GateMode::Manual,
                None,
                100,
            )
            .unwrap();

        let after_define = executor
            .storage
            .read_repo_file(".jit/gates.toml")
            .unwrap()
            .unwrap();
        assert!(
            after_define.starts_with(authored),
            "defining a gate must leave the authored registry bytes intact: {after_define:?}"
        );

        executor
            .gates()
            .remove_gate_definition(&"alpha".parse().unwrap())
            .unwrap();

        let after_remove = executor
            .storage
            .read_repo_file(".jit/gates.toml")
            .unwrap()
            .unwrap();
        for section in [
            "[[gates]]\n\
             key = \"zulu\"\n\
             title = \"Zulu\"\n\
             description = \"Workspace check\"\n\
             stage = \"postcheck\"\n\
             mode = \"manual\"\n",
            "[[gates]]\n\
             key = \"beta\"\n\
             title = \"Beta\"\n\
             description = \"Issue check\"\n\
             stage = \"postcheck\"\n\
             mode = \"manual\"\n",
        ] {
            assert!(
                after_remove.contains(section),
                "removing a gate must retain every untouched authored section: {section}"
            );
        }
        for (comment, key) in [
            (
                "# Input footprint: only files under the workspace are read.\n",
                "key = \"zulu\"",
            ),
            (
                "# Input footprint: the beta checker is issue-local.\n",
                "key = \"beta\"",
            ),
        ] {
            assert!(after_remove.contains(comment));
            assert!(after_remove.find(comment).unwrap() < after_remove.find(key).unwrap());
        }
        assert!(!after_remove.contains("key = \"alpha\""));
        assert!(
            after_remove.find("key = \"zulu\"").unwrap()
                < after_remove.find("key = \"beta\"").unwrap()
        );
        assert!(
            after_remove.find("key = \"beta\"").unwrap()
                < after_remove.find("key = \"new-gate\"").unwrap()
        );
    }

    #[test]
    fn test_gate_registry_projection_matches_edited_document_with_null_reserved_entry() {
        let mut definition = GateDefinition {
            version: 1,
            key: "nullable-extension".parse().unwrap(),
            title: "Nullable extension".to_string(),
            description: "Projection equivalence".to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            inputs: None,
            priority: 100,
            reserved: HashMap::new(),
        };
        definition
            .reserved
            .insert("discarded".to_string(), serde_json::Value::Null);
        let derived = derive_gate_registry_mutation(
            crate::declarations::GateRegistry::default(),
            &CapturedGateRegistryMutation::Define(definition),
        )
        .unwrap();

        let edited = crate::declarations::edit_gate_registry_document(None, &derived.registry)
            .expect("the proposed registry edits an empty authored document");
        let parsed =
            crate::declarations::parse_gate_registry(&edited).expect("the edited document parses");

        assert_eq!(derived.registry, parsed);
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
            let mut spec = CaptureSpec::new(
                [
                    VirtualPath::data("config.toml").unwrap(),
                    VirtualPath::data("invariants.toml").unwrap(),
                    VirtualPath::data("rules.toml").unwrap(),
                    VirtualPath::data("gates.toml").unwrap(),
                ],
                CaptureBudget {
                    max_listings: 0,
                    max_bytes: 1024 * 1024,
                    max_depth: 4,
                },
            )
            .unwrap();
            spec.discover_tail(VirtualPath::EVENTS).unwrap();
            spec
        };
        let request = CapturedGateRegistryMutation::Define(GateDefinition {
            version: 1,
            key: "ours".parse().unwrap(),
            title: "Ours".to_string(),
            description: String::new(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: HashMap::new(),
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
                crate::repository_state::RepositoryEntry::File { content, .. } => {
                    crate::declarations::parse_gate_registry(content.bytes()).unwrap()
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
            plan.delta().appended_event_bytes().unwrap().to_vec()
        };

        let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
        let first_image = first_session.capture(spec()).unwrap();
        let first_plan = derive_plan(&first_image);

        let mut concurrent = crate::declarations::GateRegistry::default();
        concurrent.gates.insert(
            "theirs".parse().unwrap(),
            GateDefinition {
                version: 1,
                key: "theirs".parse().unwrap(),
                title: "Theirs".to_string(),
                description: String::new(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
                inputs: None,
                priority: 100,
                reserved: HashMap::new(),
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
        assert!(registry.gates.contains_key(&"ours".parse().unwrap()));
        assert!(registry.gates.contains_key(&"theirs".parse().unwrap()));
        let event = storage.read_events().unwrap().pop().unwrap();
        match event.body() {
            EventBody::GateDefinitionCreated { .. } => {
                assert!(!event.id().is_empty());
                assert_ne!(event.timestamp(), chrono::DateTime::UNIX_EPOCH);
            }
            other => panic!("unexpected registry event: {other:?}"),
        }
    }

    #[test]
    fn test_gate_registry_missing_update_and_remove_keep_typed_errors() {
        for request in [
            CapturedGateRegistryMutation::Update {
                key: "missing-update".parse().unwrap(),
                update: GateUpdate {
                    title: Some("No gate".to_string()),
                    ..Default::default()
                },
            },
            CapturedGateRegistryMutation::Remove {
                key: "missing-remove".parse().unwrap(),
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
