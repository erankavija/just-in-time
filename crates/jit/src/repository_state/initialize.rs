//! Repository initialization and profile-application materialization producers.
//!
//! `repository_state` owns fresh-config rendering and the complete init/profile
//! delta. A command submits semantic choices — the template config skeleton, the
//! project name, and (for a profile) its neutral claims — and this module renders every neutral byte and composes one exact
//! [`RepositoryDelta`] over the captured base image, including the audit append.
//! The command opens the mutation session, captures the base under its guard,
//! and applies the returned plan; it never hand-builds a redo record or a
//! command-local final-byte inventory.
//!
//! The profile's asset/region/registry claims arrive as a
//! [`ProfileApplicationInput`]; this module alone derives and compares their final
//! bytes.
//! Finalization derives the coupled default-rule/schema closure and positions the
//! aggregate profile lifecycle append at the captured `events.jsonl` committed
//! length (id, timestamp, and position are never command-owned). Every byte flows
//! through `session.apply` here.

use std::collections::BTreeMap;

use crate::config::ProjectName;
use crate::config_manager::namespaces_from_config;
use crate::declarations::invariants::{serialize_invariant_registry, InvariantRegistry};
use crate::declarations::{serialize_gate_registry, GateRegistry};
use crate::domain::identity::ProfileId;
use crate::domain::{
    EventBody, ProfileLifecycleOperation, ProfileLifecycleProfile, ProfileLifecycleStatus,
};

use super::default_rules::default_ruleset;
use super::image::{
    CaptureError, DeltaError, ExpectedPreimage, FileMode, RepositoryAction, RepositoryActionOwner,
    RepositoryDelta, RepositoryEntry, RepositoryImage,
};
use super::mutation::{finalize_audit_append, fresh_index_bytes, MutationContext, MutationError};
use super::path::{RepositoryLayoutError, VirtualPath};
use super::profile_apply::ProfileTargetDecision;
use super::rule_serialize::serialize_ruleset;
use super::MaterializationDerivation;
use super::RepositoryStateError;
use super::{
    ProfileApplicationInput, ProfileContributionMaterialization, ProfileTargetDisposition,
    ProfileTargetMaterialization,
};
use crate::domain::relative_path::RootRelativePath;

/// The configuration schema version a fresh repository declares.
pub const FRESH_CONFIG_SCHEMA_VERSION: u32 = 2;

/// The configuration body a bare initialization writes.
///
/// Only what the engine derives from the repository rather than from a
/// declaration: the schema version here, and the project name
/// [`render_repo_config`] appends beneath it. A vocabulary — types, namespaces,
/// item kinds, validation defaults, a documentation-area classification —
/// reaches a repository by applying a profile package that declares it, so a
/// repository that applies none declares none.
pub fn structural_minimum_config() -> String {
    format!("[version]\nschema = {FRESH_CONFIG_SCHEMA_VERSION}\n")
}

/// Render the canonical fresh `config.toml` from a configuration body plus
/// identity.
///
/// The exact scaffold format: the body, then the project-identity table with its
/// guiding comment. Kept byte-for-byte to preserve the generated file a user
/// first sees after `jit init`. `repository_state` owns this rendering so init
/// submits the identity, never final bytes.
pub fn render_repo_config(base_config_toml: &str, project_name: &ProjectName) -> String {
    format!(
        "{}\n# =============================================================================\n# PROJECT IDENTITY\n# =============================================================================\n# Canonical, human-editable project name: the `@<project>` scope token in the\n# multi-jit addressing scheme. Must match ^[a-z][a-z0-9-]*$. Defaults to a\n# slug of this repository's directory name; edit freely.\n[project]\nname = \"{}\"\n",
        base_config_toml,
        project_name.as_str()
    )
}

/// The Git-attributes line-set claim eligibility for this initialization.
///
/// The boundary acquires typed Git evidence before init and passes it here: the
/// claim is `Eligible` only when Git identifies a containing worktree and the
/// selected data root is inside it, carrying the canonical Git-escaped
/// worktree-relative data-root events line. Otherwise it is `NotApplicable` — no
/// `.gitattributes` target is captured and init still succeeds (`@/charter/D-4`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitattributesClaim {
    /// No Git worktree, or the data root is outside it. No target is captured.
    NotApplicable,
    /// Eligible: ensure exactly this line-set (`<escaped-data-root>/events.jsonl
    /// merge=union`) is present in the worktree `.gitattributes`.
    Eligible {
        /// The canonical events merge-driver line.
        line: String,
    },
}

/// The exact outcome of the Git-attributes line-set claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GitattributesStatus {
    /// Not eligible (no Git worktree or an external data root); nothing was done.
    NotApplicable,
    /// The line-set was already present; unchanged.
    Unchanged,
    /// `.gitattributes` did not exist and was created with the jit block.
    Created,
    /// `.gitattributes` existed without the jit block and was appended to.
    Modified,
}

/// Failure while rendering or composing an initialization/profile delta.
#[derive(Debug, thiserror::Error)]
pub enum InitializationError {
    /// The worktree `.gitattributes` is occupied by an unsafe kind (symlink,
    /// directory, or unsupported), holds non-UTF-8 content, or carries a competing
    /// jit merge-driver block for a different data root.
    #[error("worktree .gitattributes cannot be safely claimed: {0}")]
    UnsafeGitattributes(String),
    /// The generated or existing configuration could not be parsed.
    #[error("failed to parse initialization configuration: {0}")]
    Config(String),
    /// Reading a captured entry failed.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// Delta normalization rejected the produced actions.
    #[error(transparent)]
    Delta(#[from] DeltaError),
    /// Path canonicalization failed.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    /// The finalizer could not compose the audit-log append.
    #[error(transparent)]
    Mutation(#[from] MutationError),
    /// A managed-document region could not be composed over a provable boundary.
    #[error(transparent)]
    ManagedDocument(#[from] super::ManagedDocumentError),
    /// Existing default-rule materialization could not be derived safely.
    #[error("failed to materialize initialization rules: {0}")]
    RuleMaterialization(String),
    /// A scaffold path is occupied by an unexpected filesystem kind.
    #[error("initialization target '{path:?}' is occupied by an unsupported filesystem kind")]
    UnexpectedOccupant {
        /// The offending path.
        path: VirtualPath,
    },
}

/// When a desired file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WritePolicy {
    /// Write only when the base captured the path as absent (preserve authored
    /// content; fill missing scaffold state).
    IfAbsent,
    /// Always write (repair-owned generated content; profile assets the
    /// derivation already proved changed).
    Always,
}

/// One desired file target with its write policy and ownership.
#[derive(Clone)]
struct DesiredFile {
    path: VirtualPath,
    bytes: Vec<u8>,
    mode: FileMode,
    policy: WritePolicy,
    owner: RepositoryActionOwner,
}

/// A fully-rendered fresh (or missing-file) repository scaffold.
///
/// `render` computes every neutral byte from the semantic choices without
/// touching the filesystem; [`delta_paths`](Self::delta_paths) enumerates every
/// path the delta may touch (so the command captures them), and
/// [`overlay_overrides`](Self::overlay_overrides) supplies the proposed bytes for
/// the command's whole-repository proposed-state validation.
pub struct InitializationScaffold {
    config: Vec<u8>,
    index: Vec<u8>,
    gates: Vec<u8>,
    invariants: Vec<u8>,
    rules: Vec<u8>,
    schemas: Vec<(String, Vec<u8>)>,
    profile: Option<ProfileApplicationInput>,
    additional_profiles: Vec<ProfileApplicationInput>,
    project_name: ProjectName,
    gitattributes: GitattributesClaim,
}

/// The marker line opening the jit-owned `.gitattributes` block.
const GITATTRIBUTES_MARKER: &str = "# JIT merge drivers";

impl InitializationScaffold {
    /// Render every neutral scaffold byte from the template body and identity.
    ///
    /// `config_skeleton` is the template's generated configuration body; the final
    /// `config.toml` is rendered here. Default rules and their schemas derive from
    /// the parsed configuration's namespaces, so the scaffold is internally
    /// coherent before any capture.
    pub fn render(
        config_skeleton: &str,
        project_name: ProjectName,
        profile: Option<ProfileApplicationInput>,
    ) -> Result<Self, InitializationError> {
        let config = render_repo_config(config_skeleton, &project_name);
        Self::from_config(config, project_name, profile)
    }

    /// Render a scaffold whose configuration bytes are already final (an existing
    /// repository's captured `config.toml`), deriving default rules/schemas from it.
    pub fn from_config(
        config: String,
        project_name: ProjectName,
        profile: Option<ProfileApplicationInput>,
    ) -> Result<Self, InitializationError> {
        let parsed =
            crate::declarations::assemble_configuration(Some(config.as_bytes()), None, None)
                .map_err(|error| InitializationError::Config(error.to_string()))?;
        let namespaces = namespaces_from_config(&parsed);
        let serialized = serialize_ruleset(&default_ruleset(&namespaces))?;
        let schemas = serialized
            .schema_files
            .into_iter()
            .map(|schema| (schema.name, schema.content.into_bytes()))
            .collect();
        Ok(Self {
            config: config.into_bytes(),
            index: fresh_index_bytes()
                .map_err(|error| InitializationError::Config(error.to_string()))?,
            gates: serialize_gate_registry(&GateRegistry::default())
                .map_err(|error| InitializationError::Config(error.to_string()))?,
            invariants: serialize_invariant_registry(&InvariantRegistry::empty())
                .map_err(|error| InitializationError::Config(error.to_string()))?,
            rules: serialized.rules_toml.into_bytes(),
            schemas,
            profile,
            additional_profiles: Vec::new(),
            project_name,
            gitattributes: GitattributesClaim::NotApplicable,
        })
    }

    /// Attach the boundary-acquired Git-attributes eligibility to this scaffold.
    pub fn with_gitattributes(mut self, claim: GitattributesClaim) -> Self {
        self.gitattributes = claim;
        self
    }

    /// Attach the dependency-first unique closure selected for initialization.
    /// The existing single-profile construction API remains useful to pure unit
    /// tests; live profiled initialization supplies its entire closure here.
    pub fn with_profiles(mut self, mut profiles: Vec<ProfileApplicationInput>) -> Self {
        self.profile = profiles.first().cloned();
        self.additional_profiles = if profiles.is_empty() {
            Vec::new()
        } else {
            profiles.split_off(1)
        };
        self
    }

    /// Whether initialization carries any profile application input.
    pub fn has_profiles(&self) -> bool {
        self.profile.is_some()
    }

    /// The complete dependency-first unique profile selection.
    pub fn profiles(&self) -> Vec<ProfileApplicationInput> {
        self.profile
            .iter()
            .cloned()
            .chain(self.additional_profiles.iter().cloned())
            .collect()
    }

    fn without_profiles(&self) -> Self {
        Self {
            config: self.config.clone(),
            index: self.index.clone(),
            gates: self.gates.clone(),
            invariants: self.invariants.clone(),
            rules: self.rules.clone(),
            schemas: self.schemas.clone(),
            profile: None,
            additional_profiles: Vec::new(),
            project_name: self.project_name.clone(),
            gitattributes: self.gitattributes.clone(),
        }
    }

    /// The eligible Git-attributes events line, if any.
    fn gitattributes_line(&self) -> Option<&str> {
        match &self.gitattributes {
            GitattributesClaim::Eligible { line } => Some(line),
            GitattributesClaim::NotApplicable => None,
        }
    }

    /// The Git-attributes claim's outcome against the captured base, for the
    /// command's report. Derived from the same pure resolution the delta uses, so
    /// the reported status and the published bytes never disagree.
    pub fn gitattributes_status(
        &self,
        base: &RepositoryImage,
    ) -> Result<GitattributesStatus, InitializationError> {
        let Some(line) = self.gitattributes_line() else {
            return Ok(GitattributesStatus::NotApplicable);
        };
        let entry = base.entry(&VirtualPath::GITATTRIBUTES)?;
        Ok(resolve_gitattributes(entry, line)?.0)
    }

    /// Whether the scaffold carries a profile contribution.
    pub fn profile(&self) -> Option<&ProfileApplicationInput> {
        self.profile.as_ref()
    }

    /// Every desired neutral (and profile) file, with its write policy.
    fn desired_files(&self) -> Result<Vec<DesiredFile>, InitializationError> {
        let mut files = vec![
            DesiredFile {
                path: VirtualPath::CONFIG,
                bytes: self.config.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: RepositoryActionOwner::RepositoryInit,
            },
            DesiredFile {
                path: VirtualPath::INDEX,
                bytes: self.index.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: RepositoryActionOwner::RepositoryInit,
            },
            DesiredFile {
                path: VirtualPath::GATES,
                bytes: self.gates.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: RepositoryActionOwner::RepositoryInit,
            },
            DesiredFile {
                path: VirtualPath::INVARIANTS,
                bytes: self.invariants.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: RepositoryActionOwner::RepositoryInit,
            },
            DesiredFile {
                path: VirtualPath::RULES,
                bytes: self.rules.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: RepositoryActionOwner::RepositoryInit,
            },
        ];
        for (name, bytes) in &self.schemas {
            files.push(DesiredFile {
                path: VirtualPath::schema(name)?,
                bytes: bytes.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::Always,
                owner: RepositoryActionOwner::RepositoryInit,
            });
        }
        // The audit log is not a neutral scaffold file: the finalizer positions an
        // aggregate lifecycle append for a profiled init at its captured committed
        // length. A plain init creates an absent log empty. See `derive_initialization`.
        Ok(files)
    }

    /// Directories the scaffold must create that hold no delta file. Only the
    /// issues directory qualifies; schema directories are ancestors of
    /// written files and enter the directory closure automatically.
    fn explicit_dirs(&self) -> Result<Vec<VirtualPath>, InitializationError> {
        Ok(vec![VirtualPath::ISSUES])
    }

    /// Every path the composed delta may touch — files, the
    /// explicit directories, and every ancestor directory of those — so the command
    /// discovers them into its capture spec and `apply` finds each action's
    /// captured preimage. Over-inclusion is safe; a missing delta path is not.
    pub fn delta_paths(&self) -> Result<Vec<VirtualPath>, InitializationError> {
        let mut paths: Vec<VirtualPath> = self
            .desired_files()?
            .into_iter()
            .map(|file| file.path)
            .collect();
        // The finalizer positions its append at `events.jsonl`'s committed length;
        // the tail probe captures enough to derive that offset without reading the
        // whole log.
        paths.push(VirtualPath::EVENTS);
        for profile in self.profiles() {
            paths.extend(profile.claims.target_paths());
            if profile.owns_default_rule_authority() {
                paths.push(VirtualPath::SCHEMAS);
            }
        }
        if self.gitattributes_line().is_some() {
            paths.push(VirtualPath::GITATTRIBUTES);
        }
        // The data root's ignore file is composed against the captured base by
        // the same producer derived-state repair reaches.
        paths.push(VirtualPath::DATA_IGNORE);
        paths.extend(self.explicit_dirs()?);
        with_ancestor_dirs(paths)
    }

    /// The proposed bytes for every scaffold and profile target, for the command's
    /// whole-repository proposed-state validation overlay.
    pub fn overlay_overrides(
        &self,
    ) -> Result<BTreeMap<VirtualPath, Option<Vec<u8>>>, InitializationError> {
        let overrides: BTreeMap<VirtualPath, Option<Vec<u8>>> = self
            .desired_files()?
            .into_iter()
            .map(|file| (file.path, Some(file.bytes)))
            .collect();
        Ok(overrides)
    }

    /// Enumerate the profile render closure implied by this scaffold over `base`.
    ///
    /// Unlike [`Self::overlay_overrides`], this applies `IfAbsent` policy before
    /// deriving declarations, so a re-init closes over preserved authored files
    /// rather than the neutral defaults used only by the first probe capture.
    pub fn profile_capture_closure(
        &self,
        base: &RepositoryImage,
    ) -> Result<Vec<VirtualPath>, InitializationError> {
        if !self.has_profiles() {
            return Ok(Vec::new());
        }
        let profiles = self.profiles();
        let neutral = self.profile_composition_base(base)?;
        let contribution_context = profiles
            .iter()
            .flat_map(|profile| profile.contribution_context.iter().cloned())
            .collect::<Vec<_>>();
        let contribution_base = super::apply_overlay(
            &neutral,
            super::profile_contribution_overrides(&neutral, &contribution_context)
                .map_err(rule_materialization_error)?,
        )
        .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))?;
        profiles
            .iter()
            .map(|profile| {
                super::profile_apply::profile_capture_closure(&contribution_base, profile)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|closures| closures.into_iter().flatten().collect())
            .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))
    }

    /// Overlay only the neutral scaffold files that initialization will actually
    /// publish, preserving existing `IfAbsent` authored files. This is the
    /// repository view profile composition is captured against before the
    /// scaffold can publish.
    fn profile_composition_base(
        &self,
        base: &RepositoryImage,
    ) -> Result<RepositoryImage, InitializationError> {
        super::apply_overlay(base, desired_overrides(base, &self.desired_files()?)?)
            .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))
    }
}

/// Combine neutral initialization and an aggregate profile closure into one
/// final delta. Both partial derivations remain pure proposed-state planning;
/// this function rebuilds every write with the original captured preimage so
/// one mutation session can publish the entire repository image atomically.
fn derive_initialization_with_profiles(
    base: &RepositoryImage,
    scaffold: &InitializationScaffold,
    context: &MutationContext,
) -> Result<MaterializationDerivation, InitializationError> {
    let neutral = derive_initialization(base, &scaffold.without_profiles(), context)?;
    let neutral_image = apply_action_overlay(base, neutral.delta.actions())?;
    let profiles = scaffold.profiles();
    let forced_changed = coupled_default_repair_profile(neutral.delta.actions(), &profiles)
        .map(|profile| std::collections::BTreeSet::from([profile.id.clone()]))
        .unwrap_or_default();
    let applications = derive_profile_applications(
        &neutral_image,
        &profiles,
        context,
        ProfileLifecycleOperation::Initialize,
        forced_changed,
    )?;
    let applied_profiles = applications.applied_profiles;
    let mut actions = BTreeMap::new();
    for action in neutral
        .delta
        .actions()
        .iter()
        .chain(applications.delta.actions())
    {
        fold_rebased_action(base, &mut actions, action)?;
    }
    for action in directory_actions(
        base,
        &actions.values().cloned().collect::<Vec<_>>(),
        &scaffold.explicit_dirs()?,
    )? {
        fold_rebased_action(base, &mut actions, &action)?;
    }
    let delta = RepositoryDelta::new(base.layout(), actions.into_values().collect())?;
    Ok(MaterializationDerivation::new(delta)
        .with_profile_targets(applications.profile_targets)
        .with_profile_contributions(applications.profile_contributions)
        .with_applied_profiles(applied_profiles))
}

/// Attribute neutral default-rule/schema repair to the first dependency-first
/// profile that owns the authored rule authority. The neutral scaffold derives
/// this coupled state before aggregate candidates see it, so without this handoff
/// a real repair would publish with neither that profile's result nor its current
/// historical audit entry.
fn coupled_default_repair_profile<'a>(
    actions: &[RepositoryAction],
    profiles: &'a [ProfileApplicationInput],
) -> Option<&'a ProfileApplicationInput> {
    actions
        .iter()
        .any(|action| {
            matches!(
                action.owner(),
                RepositoryActionOwner::DefaultRuleMembership | RepositoryActionOwner::DefaultSchema
            )
        })
        .then(|| {
            profiles
                .iter()
                .find(|profile| profile.owns_default_rule_authority())
        })
        .flatten()
}

/// Compose the complete initialization delta over the captured base image.
///
/// Every action's expected preimage is derived from the captured base, so
/// `session.apply` revalidates against exactly what was captured. Neutral scaffold
/// files fill missing state (authored content preserved), generated schemas are
/// rewritten, and profile assets write unconditionally (the derivation proved them changed). The audit log is composed
/// by the finalizer (`context`): a plain init creates it empty, a profiled init
/// appends one aggregate lifecycle record. The data root's ignore file is composed
/// by the producer derived-state repair reaches, so it lands before the first
/// commit point is ever written beneath the data root. An empty delta (nothing to
/// publish) is a complete no-op.
pub(super) fn derive_initialization(
    base: &RepositoryImage,
    scaffold: &InitializationScaffold,
    context: &MutationContext,
) -> Result<MaterializationDerivation, InitializationError> {
    if scaffold.has_profiles() {
        return derive_initialization_with_profiles(base, scaffold, context);
    }
    let mut actions = Vec::new();
    let config_path = VirtualPath::CONFIG;
    let rules_path = VirtualPath::RULES;
    let existing_rules = matches!(base.entry(&rules_path)?, RepositoryEntry::File { .. });
    let compose_rules = existing_rules;
    let generated_paths: std::collections::BTreeSet<VirtualPath> = scaffold
        .schemas
        .iter()
        .map(|(name, _)| VirtualPath::schema(name))
        .collect::<Result<_, _>>()?;
    let desired = dedup_last_wins(scaffold.desired_files()?);
    let final_authority = authored_rule_overrides(base, &desired, &config_path, &rules_path)?;
    let publishable = desired
        .clone()
        .into_iter()
        .filter(|file| !compose_rules || !generated_paths.contains(&file.path))
        .collect::<Vec<_>>();
    push_file_actions(base, &publishable, &mut actions)?;
    if compose_rules {
        compose_existing_default_rules(base, final_authority, &mut actions)?;
    }
    if let Some(action) = events_action(base)? {
        actions.push(action);
    }
    push_gitattributes_action(base, scaffold, &mut actions)?;
    actions.extend(super::materialize::compose_data_root_ignore(base)?);
    let mut all = directory_actions(base, &actions, &scaffold.explicit_dirs()?)?;
    all.extend(actions);
    let delta = RepositoryDelta::new(base.layout(), all)?;
    Ok(MaterializationDerivation::new(delta))
}

fn desired_overrides(
    base: &RepositoryImage,
    desired: &[DesiredFile],
) -> Result<BTreeMap<VirtualPath, Option<Vec<u8>>>, InitializationError> {
    desired
        .iter()
        .try_fold(BTreeMap::new(), |mut overlay, file| {
            if file.policy == WritePolicy::Always
                || matches!(base.entry(&file.path)?, RepositoryEntry::Absent)
            {
                overlay.insert(file.path.clone(), Some(file.bytes.clone()));
            }
            Ok(overlay)
        })
}

/// Select the final authored config/rules bytes before derived materialization.
///
/// Existing authored files fall through to `base`; an absent scaffold target or
/// an always-written profile target becomes an overlay. Schema files are
/// deliberately excluded because they are outputs of the rules materializer.
fn authored_rule_overrides(
    base: &RepositoryImage,
    desired: &[DesiredFile],
    config_path: &VirtualPath,
    rules_path: &VirtualPath,
) -> Result<BTreeMap<VirtualPath, Option<Vec<u8>>>, InitializationError> {
    desired
        .iter()
        .filter(|file| file.path == *config_path || file.path == *rules_path)
        .try_fold(BTreeMap::new(), |mut overrides, file| {
            let absent = matches!(base.entry(&file.path)?, RepositoryEntry::Absent);
            if absent || file.policy == WritePolicy::Always {
                overrides.insert(file.path.clone(), Some(file.bytes.clone()));
            }
            Ok(overrides)
        })
}

/// Compose default rules from the final authored config/rules image and merge the
/// derived writes back against the captured base preimages.
///
/// The caller excludes scaffold schema actions first. A derived action replaces
/// any raw profile/scaffold action for the same target; an unchanged authored
/// rules file keeps its raw action so a fresh repository still publishes it.
fn compose_existing_default_rules(
    base: &RepositoryImage,
    final_authority: BTreeMap<VirtualPath, Option<Vec<u8>>>,
    actions: &mut Vec<RepositoryAction>,
) -> Result<(), InitializationError> {
    let proposed = super::apply_overlay(base, final_authority)
        .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))?;
    let config = super::materialize::assemble_config(&proposed)
        .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))?;
    let derived = super::materialize::compose_default_ruleset(&proposed, &config)
        .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))?;
    for action in derived {
        actions.retain(|existing| existing.path() != action.path());
        actions.push(rebase_action(base, action)?);
    }
    Ok(())
}

/// Replace an action's proposed-image precondition with the captured base
/// precondition while preserving its exact desired effect and owner.
fn rebase_action(
    base: &RepositoryImage,
    mut action: RepositoryAction,
) -> Result<RepositoryAction, InitializationError> {
    let rebased = ExpectedPreimage::of(base.entry(action.path())?);
    match &mut action {
        RepositoryAction::CreateDirectory { expected, .. }
        | RepositoryAction::WriteFile { expected, .. }
        | RepositoryAction::SetMode { expected, .. }
        | RepositoryAction::DeleteFile { expected, .. }
        | RepositoryAction::AppendEvents { expected, .. } => *expected = rebased,
    }
    Ok(action)
}

/// Project the byte-bearing effects of exact actions into the existing proposed
/// image. Directory creation and mode-only changes remain in the delta; they do
/// not change bytes that a later profile derivation can observe.
fn apply_action_overlay(
    base: &RepositoryImage,
    actions: &[RepositoryAction],
) -> Result<RepositoryImage, InitializationError> {
    super::apply_overlay(base, super::action_overrides(actions))
        .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))
}

/// Keep the final effect for one canonical target and derive its preimage from
/// the original captured image. A deletion of a file created earlier in this
/// aggregate has no final effect, while a later mode-only action preserves the
/// earlier aggregate write's bytes.
fn fold_rebased_action(
    base: &RepositoryImage,
    actions: &mut BTreeMap<VirtualPath, RepositoryAction>,
    action: &RepositoryAction,
) -> Result<(), InitializationError> {
    let path = action.path().clone();
    if matches!(action, RepositoryAction::DeleteFile { .. })
        && matches!(base.entry(&path)?, RepositoryEntry::Absent)
    {
        actions.remove(&path);
        return Ok(());
    }
    if let (
        RepositoryAction::SetMode { owner, mode, .. },
        Some(RepositoryAction::WriteFile {
            path,
            bytes,
            expected,
            ..
        }),
    ) = (action, actions.get(&path))
    {
        actions.insert(
            path.clone(),
            RepositoryAction::WriteFile {
                path: path.clone(),
                owner: *owner,
                expected: expected.clone(),
                bytes: bytes.clone(),
                mode: *mode,
            },
        );
        return Ok(());
    }
    actions.insert(path, rebase_action(base, action.clone())?);
    Ok(())
}

/// Compose the neutral initialization `events.jsonl` action.
///
/// A plain initialization creates the log empty when absent and leaves it untouched
/// when already present. Profiled initialization takes the aggregate derivation,
/// which appends its current per-profile historical events through the finalizer.
fn events_action(base: &RepositoryImage) -> Result<Option<RepositoryAction>, InitializationError> {
    let path = VirtualPath::EVENTS;
    match base.entry(&path)? {
        RepositoryEntry::Absent => Ok(Some(RepositoryAction::AppendEvents {
            owner: RepositoryActionOwner::RepositoryInit,
            expected: ExpectedPreimage::Absent,
            offset: 0,
            bytes: Vec::new(),
        })),
        RepositoryEntry::File { .. } | RepositoryEntry::FileTail { .. } => Ok(None),
        _ => Err(InitializationError::UnexpectedOccupant { path }),
    }
}

/// Emit the worktree `.gitattributes` write when the claim is eligible and the
/// captured content is missing the jit line-set, rejecting an unsafe occupant.
fn push_gitattributes_action(
    base: &RepositoryImage,
    scaffold: &InitializationScaffold,
    actions: &mut Vec<RepositoryAction>,
) -> Result<(), InitializationError> {
    let Some(line) = scaffold.gitattributes_line() else {
        return Ok(());
    };
    let path = VirtualPath::GITATTRIBUTES;
    let entry = base.entry(&path)?;
    if let (_status, Some(bytes)) = resolve_gitattributes(entry, line)? {
        actions.push(RepositoryAction::WriteFile {
            path,
            owner: RepositoryActionOwner::GitattributesMergeDriver,
            expected: ExpectedPreimage::of(entry),
            bytes,
            mode: FileMode::Regular,
        });
    }
    Ok(())
}

/// Resolve the Git-attributes claim against the captured `.gitattributes` entry:
/// its status and, when a write is required, the exact desired bytes preserving
/// every unrelated byte.
///
/// An absent file is created with the jit block; an existing file already holding
/// the exact marker line followed by the exact rule line is unchanged; a file
/// missing the block gets it appended (with the required separator). A non-UTF-8
/// file, a jit block present for a DIFFERENT data root (competing claim), or a
/// symlink/directory/unsupported occupant is a typed error aborting init before
/// anything is published.
fn resolve_gitattributes(
    entry: &RepositoryEntry,
    line: &str,
) -> Result<(GitattributesStatus, Option<Vec<u8>>), InitializationError> {
    let block = format!("{GITATTRIBUTES_MARKER}\n{line}\n");
    match entry {
        RepositoryEntry::Absent => Ok((GitattributesStatus::Created, Some(block.into_bytes()))),
        RepositoryEntry::File { content, .. } => {
            let content = std::str::from_utf8(content.bytes()).map_err(|_| {
                InitializationError::UnsafeGitattributes(".gitattributes is not valid UTF-8".into())
            })?;
            let mut lines = content.lines().peekable();
            let mut claim_is_installed = false;
            while let Some(candidate) = lines.next() {
                if candidate != GITATTRIBUTES_MARKER {
                    continue;
                }
                if lines.peek().copied() != Some(line) {
                    return Err(InitializationError::UnsafeGitattributes(
                        "a jit merge-driver block is present for a different data root".into(),
                    ));
                }
                claim_is_installed = true;
            }
            if claim_is_installed {
                Ok((GitattributesStatus::Unchanged, None))
            } else {
                let separator = if content.ends_with('\n') {
                    "\n"
                } else {
                    "\n\n"
                };
                Ok((
                    GitattributesStatus::Modified,
                    Some(format!("{content}{separator}{block}").into_bytes()),
                ))
            }
        }
        _ => Err(InitializationError::UnsafeGitattributes(
            "occupied by an unsupported filesystem kind".into(),
        )),
    }
}

/// Derive the uncommitted candidate for one member of an aggregate profile
/// selection. The caller folds candidates over a proposed image and rebuilds
/// their final writes against the original captured base, so this helper never
/// becomes a per-package publication seam.
fn derive_profile_application_candidate(
    base: &RepositoryImage,
    profile: &ProfileApplicationInput,
    contested_files: &BTreeMap<VirtualPath, Vec<ProfileId>>,
) -> Result<MaterializationDerivation, InitializationError> {
    let composed = super::profile_apply::compose_profile_targets_with_context(
        base,
        profile.claims.clone(),
        profile.contribution_context.clone(),
        contested_files,
    )
    .map_err(rule_materialization_error)?;
    let contributions = composed
        .contribution_decisions
        .iter()
        .map(
            |(identity, disposition)| ProfileContributionMaterialization {
                owner: profile.id.clone(),
                identity: identity.clone(),
                disposition: disposition.clone(),
            },
        )
        .collect::<Vec<_>>();
    let publishable = composed
        .decisions
        .iter()
        .filter(|(_, decision)| {
            matches!(
                decision.disposition,
                ProfileTargetDisposition::Create | ProfileTargetDisposition::Update
            )
        })
        .map(|(path, _)| path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let files = composed
        .targets
        .into_iter()
        .filter(|(path, _)| publishable.contains(path))
        .map(|(path, (bytes, mode))| DesiredFile {
            path,
            bytes,
            mode,
            policy: WritePolicy::Always,
            owner: RepositoryActionOwner::ProfileApplication,
        })
        .collect::<Vec<_>>();
    let mut actions = Vec::new();
    push_file_actions(base, &files, &mut actions)?;
    let config_path = VirtualPath::CONFIG;
    let rules_path = VirtualPath::RULES;
    if profile.owns_default_rule_authority() {
        let authority = authored_rule_overrides(base, &files, &config_path, &rules_path)?;
        compose_existing_default_rules(base, authority, &mut actions)?;
    }
    let mut all = directory_actions(base, &actions, &[])?;
    all.extend(actions);
    let delta = RepositoryDelta::new(base.layout(), all)?;
    // The delta decides every file this candidate writes, the derived ones
    // included, so what a report states and what is written are one answer. What
    // the delta leaves out keeps the composition's decision.
    let mut decisions = composed.decisions;
    for action in delta.file_writes() {
        let path = action.path();
        let conflicted = matches!(
            decisions.get(path),
            Some(decision) if matches!(decision.disposition, ProfileTargetDisposition::Conflict(_))
        );
        if !conflicted {
            decisions.insert(path.clone(), written_file_decision(base, action)?);
        }
    }
    let targets = decisions
        .into_iter()
        .map(|(path, decision)| ProfileTargetMaterialization {
            owner: profile.id.clone(),
            path,
            disposition: decision.disposition,
            mode: decision.mode,
        })
        .collect();
    Ok(MaterializationDerivation::new(delta)
        .with_profile_targets(targets)
        .with_profile_contributions(contributions))
}

/// What one of [`RepositoryDelta::file_writes`] does to its file.
fn written_file_decision(
    base: &RepositoryImage,
    action: &RepositoryAction,
) -> Result<ProfileTargetDecision, InitializationError> {
    let held = !matches!(base.entry(action.path())?, RepositoryEntry::Absent);
    Ok(match action {
        RepositoryAction::WriteFile { mode, .. } | RepositoryAction::SetMode { mode, .. } => {
            ProfileTargetDecision {
                disposition: if held {
                    ProfileTargetDisposition::Update
                } else {
                    ProfileTargetDisposition::Create
                },
                mode: *mode,
            }
        }
        _ => ProfileTargetDecision {
            disposition: ProfileTargetDisposition::Remove,
            mode: FileMode::Regular,
        },
    })
}

/// Compose one profile selection and record its aggregate lifecycle outcome.
///
/// `operation` names the lifecycle command that requested the selection; each
/// member's own status is derived from what its candidate publishes rather than
/// from that name. `forced_changed` carries coupled initialization materializations
/// that are derived before the profile candidates themselves. Keeping both
/// facts in this planner means initialization and every lifecycle command share
/// one composition and one event shape.
pub(super) fn derive_profile_applications(
    base: &RepositoryImage,
    profiles: &[ProfileApplicationInput],
    context: &MutationContext,
    operation: ProfileLifecycleOperation,
    forced_changed: std::collections::BTreeSet<ProfileId>,
) -> Result<MaterializationDerivation, InitializationError> {
    let mut proposed = base.clone();
    let mut actions = BTreeMap::new();
    let mut targets = Vec::new();
    let mut contributions = Vec::new();
    let mut lifecycle_profiles = Vec::with_capacity(profiles.len());
    let mut applied_profiles = forced_changed.clone();
    let mut aggregate_changed = !forced_changed.is_empty();
    let contested_files = super::profile_apply::contested_file_targets(
        profiles.iter().map(|profile| &profile.claims),
    );

    for profile in profiles {
        let candidate = derive_profile_application_candidate(&proposed, profile, &contested_files)?;
        let candidate_actions = candidate
            .delta
            .actions()
            .iter()
            .filter(|action| action.path() != &VirtualPath::EVENTS)
            .cloned()
            .collect::<Vec<_>>();
        let changed = forced_changed.contains(&profile.id) || !candidate_actions.is_empty();
        aggregate_changed |= !candidate_actions.is_empty();
        for action in &candidate_actions {
            fold_rebased_action(base, &mut actions, action)?;
        }
        proposed = apply_action_overlay(&proposed, &candidate_actions)?;
        targets.extend(candidate.profile_targets);
        contributions.extend(candidate.profile_contributions);
        if changed {
            applied_profiles.insert(profile.id.clone());
        }
        lifecycle_profiles.push(profile_lifecycle_profile(profile, changed));
    }

    if aggregate_changed {
        let event = EventBody::draft_profile_lifecycle(operation, lifecycle_profiles);
        if let Some(action) = finalize_audit_append(base, context, vec![(2, event)])? {
            fold_rebased_action(base, &mut actions, &action)?;
        }
    }
    for action in directory_actions(base, &actions.values().cloned().collect::<Vec<_>>(), &[])? {
        fold_rebased_action(base, &mut actions, &action)?;
    }
    let delta = RepositoryDelta::new(base.layout(), actions.into_values().collect())?;
    Ok(MaterializationDerivation::new(delta)
        .with_profile_targets(targets)
        .with_profile_contributions(contributions)
        .with_applied_profiles(applied_profiles))
}

/// Classify one profile's aggregate outcome from whether its candidate
/// publishes anything: a publishing candidate installs the package's content,
/// and one that publishes nothing leaves the repository unchanged.
fn profile_lifecycle_profile(
    profile: &ProfileApplicationInput,
    changed: bool,
) -> ProfileLifecycleProfile {
    let status = if changed {
        ProfileLifecycleStatus::Installed
    } else {
        ProfileLifecycleStatus::Unchanged
    };
    ProfileLifecycleProfile {
        id: profile.id.clone(),
        version: profile.version.clone(),
        status,
        variables: profile.variables.values(),
    }
}

/// Deduplicate desired files by canonical path, keeping the LAST occurrence so a
/// profile asset target (appended after the neutral scaffold) overrides a
/// colliding neutral file — the fresh dogfood profile, for instance, supplies the
/// authoritative `config.toml`. Preserves the neutral-then-profile order otherwise.
fn dedup_last_wins(files: Vec<DesiredFile>) -> Vec<DesiredFile> {
    let mut ordered: BTreeMap<VirtualPath, DesiredFile> = BTreeMap::new();
    for file in files {
        ordered.insert(file.path.clone(), file);
    }
    ordered.into_values().collect()
}

/// Carry a producer failure raised while composing a profile's targets as an
/// initialization failure.
fn rule_materialization_error(error: RepositoryStateError) -> InitializationError {
    InitializationError::RuleMaterialization(error.to_string())
}

/// Emit `CreateDirectory` actions for every ancestor directory of the written
/// files plus the explicit directories, in canonical order, skipping directories
/// the base already captured and rejecting a non-directory occupant.
///
/// The delta model requires an explicit `CreateDirectory` for each level: both
/// backends verify a write target's parent is a captured directory rather than
/// creating ancestors implicitly, so a profile asset nested under new worktree
/// directories needs each level created in the same delta.
fn directory_actions(
    base: &RepositoryImage,
    file_actions: &[RepositoryAction],
    explicit: &[VirtualPath],
) -> Result<Vec<RepositoryAction>, InitializationError> {
    let mut targets: std::collections::BTreeSet<VirtualPath> = explicit.iter().cloned().collect();
    for action in file_actions {
        if let RepositoryAction::WriteFile { path, .. } = action {
            targets.extend(ancestor_dirs(path)?);
        }
    }
    for dir in explicit {
        targets.extend(ancestor_dirs(dir)?);
    }
    let mut actions = Vec::new();
    for dir in targets {
        match base.entry(&dir)? {
            RepositoryEntry::Absent => actions.push(RepositoryAction::CreateDirectory {
                path: dir,
                owner: RepositoryActionOwner::RepositoryInit,
                expected: ExpectedPreimage::Absent,
            }),
            RepositoryEntry::Directory { .. } => {}
            _ => return Err(InitializationError::UnexpectedOccupant { path: dir }),
        }
    }
    Ok(actions)
}

/// Extend `paths` with every ancestor directory of each path, deduplicated.
fn with_ancestor_dirs(paths: Vec<VirtualPath>) -> Result<Vec<VirtualPath>, InitializationError> {
    let mut closure: std::collections::BTreeSet<VirtualPath> = paths.iter().cloned().collect();
    for path in &paths {
        closure.extend(ancestor_dirs(path)?);
    }
    Ok(closure.into_iter().collect())
}

/// The ancestor directories of `path` within its root (excluding the root itself),
/// deepest first; order is immaterial because the delta re-sorts by path.
fn ancestor_dirs(path: &VirtualPath) -> Result<Vec<VirtualPath>, RepositoryLayoutError> {
    let mut dirs = Vec::new();
    let mut parent = path.relative().as_path().parent();
    while let Some(component) = parent {
        if component.as_os_str().is_empty() {
            break;
        }
        dirs.push(VirtualPath::from_root(
            path.root_class(),
            RootRelativePath::parse(component)?,
        )?);
        parent = component.parent();
    }
    Ok(dirs)
}

/// Emit `WriteFile` actions honoring each file's write policy, deriving the
/// expected preimage from the captured base and rejecting non-file occupants.
fn push_file_actions(
    base: &RepositoryImage,
    files: &[DesiredFile],
    actions: &mut Vec<RepositoryAction>,
) -> Result<(), InitializationError> {
    for file in files {
        let entry = base.entry(&file.path)?;
        match entry {
            RepositoryEntry::Absent | RepositoryEntry::File { .. } => {}
            _ => {
                return Err(InitializationError::UnexpectedOccupant {
                    path: file.path.clone(),
                })
            }
        }
        let write = match file.policy {
            WritePolicy::IfAbsent => matches!(entry, RepositoryEntry::Absent),
            WritePolicy::Always => true,
        };
        if write {
            actions.push(RepositoryAction::WriteFile {
                path: file.path.clone(),
                owner: file.owner,
                expected: ExpectedPreimage::of(entry),
                bytes: file.bytes.clone(),
                mode: file.mode,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{CapturedBytes, EntryIdentity};

    const LINE: &str = ".jit/events.jsonl merge=union";

    fn file(bytes: &[u8]) -> RepositoryEntry {
        RepositoryEntry::File {
            content: CapturedBytes::new("obj", bytes.to_vec()).unwrap(),
            mode: FileMode::Regular,
        }
    }

    fn profile_input(id: &str) -> ProfileApplicationInput {
        ProfileApplicationInput {
            id: ProfileId::try_from(id).unwrap(),
            version: semver::Version::new(1, 0, 0),
            variables: serde_json::from_value(serde_json::json!({
                "NAME": {"value": "value", "source": "set"}
            }))
            .unwrap(),
            targets: Default::default(),
            claims: crate::repository_state::ProfileClaims {
                package_id: ProfileId::try_from(id).unwrap(),
                contributions: Vec::new(),
                assets: Vec::new(),
                regions: Vec::new(),
            },
            contribution_context: Vec::new(),
        }
    }

    #[test]
    fn test_profile_lifecycle_profile_classifies_status_from_whether_the_candidate_publishes() {
        let profile = profile_input("workflow");

        let published = profile_lifecycle_profile(&profile, true);
        let unpublished = profile_lifecycle_profile(&profile, false);

        assert_eq!(published.status, ProfileLifecycleStatus::Installed);
        assert_eq!(unpublished.status, ProfileLifecycleStatus::Unchanged);
        assert_eq!(published.variables, unpublished.variables);
        assert_eq!(published.id, profile.id);
    }

    #[test]
    fn test_profile_lifecycle_profile_records_the_package_version_and_applied_values() {
        let profile = profile_input("workflow");

        let recorded = profile_lifecycle_profile(&profile, true);

        assert_eq!(recorded.version, profile.version);
        assert_eq!(recorded.variables, profile.variables.values());
    }

    #[test]
    fn test_resolve_gitattributes_absent_creates_block() {
        let (status, bytes) = resolve_gitattributes(&RepositoryEntry::Absent, LINE).unwrap();
        assert_eq!(status, GitattributesStatus::Created);
        assert_eq!(
            bytes.unwrap(),
            format!("# JIT merge drivers\n{LINE}\n").into_bytes()
        );
    }

    #[test]
    fn test_resolve_gitattributes_exact_owned_block_is_unchanged() {
        let entry = file(format!("# JIT merge drivers\n{LINE}\n").as_bytes());
        let (status, bytes) = resolve_gitattributes(&entry, LINE).unwrap();
        assert_eq!(status, GitattributesStatus::Unchanged);
        assert!(bytes.is_none());
    }

    #[test]
    fn test_resolve_gitattributes_commented_rule_does_not_satisfy_claim() {
        let content = format!("# {LINE}\n");
        let entry = file(content.as_bytes());
        let (status, bytes) = resolve_gitattributes(&entry, LINE).unwrap();
        assert_eq!(status, GitattributesStatus::Modified);
        assert_eq!(
            bytes.unwrap(),
            format!("{content}\n# JIT merge drivers\n{LINE}\n").into_bytes()
        );
    }

    #[test]
    fn test_resolve_gitattributes_rule_substrings_do_not_satisfy_claim() {
        for content in [
            format!("prefix {LINE}\n"),
            format!("{LINE} # unrelated suffix\n"),
        ] {
            let entry = file(content.as_bytes());
            let (status, bytes) = resolve_gitattributes(&entry, LINE).unwrap();
            assert_eq!(status, GitattributesStatus::Modified);
            assert_eq!(
                bytes.unwrap(),
                format!("{content}\n# JIT merge drivers\n{LINE}\n").into_bytes()
            );
        }
    }

    #[test]
    fn test_resolve_gitattributes_commented_marker_does_not_claim_rule() {
        let content = format!("# {GITATTRIBUTES_MARKER}\n{LINE}\n");
        let entry = file(content.as_bytes());
        let (status, bytes) = resolve_gitattributes(&entry, LINE).unwrap();
        assert_eq!(status, GitattributesStatus::Modified);
        assert_eq!(
            bytes.unwrap(),
            format!("{content}\n# JIT merge drivers\n{LINE}\n").into_bytes()
        );
    }

    #[test]
    fn test_resolve_gitattributes_appends_preserving_unrelated_bytes() {
        let entry = file(b"*.txt text\n");
        let (status, bytes) = resolve_gitattributes(&entry, LINE).unwrap();
        assert_eq!(status, GitattributesStatus::Modified);
        assert_eq!(
            bytes.unwrap(),
            format!("*.txt text\n\n# JIT merge drivers\n{LINE}\n").into_bytes()
        );
    }

    #[test]
    fn test_resolve_gitattributes_inserts_separator_when_prefix_unterminated() {
        let entry = file(b"*.txt text");
        let (_status, bytes) = resolve_gitattributes(&entry, LINE).unwrap();
        assert_eq!(
            bytes.unwrap(),
            format!("*.txt text\n\n# JIT merge drivers\n{LINE}\n").into_bytes()
        );
    }

    #[test]
    fn test_resolve_gitattributes_competing_block_for_other_root_is_error() {
        let entry = file(b"# JIT merge drivers\nother/events.jsonl merge=union\n");
        assert!(matches!(
            resolve_gitattributes(&entry, LINE),
            Err(InitializationError::UnsafeGitattributes(_))
        ));
    }

    #[test]
    fn test_resolve_gitattributes_non_utf8_is_error() {
        let entry = file(&[0xff, 0xfe, 0x00]);
        assert!(matches!(
            resolve_gitattributes(&entry, LINE),
            Err(InitializationError::UnsafeGitattributes(_))
        ));
    }

    #[test]
    fn test_resolve_gitattributes_unsupported_occupant_is_error() {
        let entry = RepositoryEntry::Directory {
            identity: EntryIdentity::for_bytes("obj", b"dir").unwrap(),
            mode: FileMode::Regular,
        };
        assert!(matches!(
            resolve_gitattributes(&entry, LINE),
            Err(InitializationError::UnsafeGitattributes(_))
        ));
    }
}
