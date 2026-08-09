//! Repository initialization and profile-application materialization producers.
//!
//! `repository_state` owns fresh-config rendering and the complete init/profile
//! delta. A command submits semantic choices — the template config skeleton, the
//! project name, and (for a profile) its neutral claims and provenance
//! metadata — and this module renders every neutral byte and composes one exact
//! [`RepositoryDelta`] over the captured base image, including the audit append.
//! The command opens the recovered session, captures the base under its guard,
//! and applies the returned plan; it never hand-builds a transaction or a
//! command-local final-byte inventory.
//!
//! The profile's asset/region/registry claims arrive as a
//! [`ProfileApplicationInput`]; this module alone derives and compares their final
//! bytes.
//! Finalization derives the coupled default-rule/schema closure and composes the
//! `ProfileApplied` line from the captured `events.jsonl` prefix (id, timestamp,
//! and torn-tail handling are never command-owned). Every byte flows through
//! `session.apply` here.

use std::collections::BTreeMap;

use crate::config::ProjectName;
use crate::config_manager::namespaces_from_config;
use crate::declarations::invariants::{serialize_invariant_registry, InvariantRegistry};
use crate::declarations::{serialize_gate_registry, GateRegistry};

use super::default_rules::default_ruleset;
use super::image::{
    CaptureError, DeltaError, ExpectedPreimage, FileMode, MaterializationIntent, PlanHashError,
    RepositoryAction, RepositoryDelta, RepositoryEntry, RepositoryImage, RepositorySeed,
    RepositorySeedKind, SeedError,
};
use super::mutation::{
    finalize_audit_append, fresh_index_bytes, profile_applied_event, MutationContext, MutationError,
};
use super::path::{RepositoryLayoutError, RootRelativePath, VirtualPath};
use super::rule_serialize::serialize_ruleset;
use super::MaterializationDerivation;
use super::{
    AppliedProfileRecord, ContributionCompositionConflict, ProfileApplicationInput,
    ProfileTargetDisposition, ProfileTargetMaterialization,
};
use super::{ProfileTargetConflictError, RepositoryStateError};

/// Stable ownership identity for the neutral scaffold materialization.
const SCAFFOLD_OWNER: &str = "repository-init";
/// Stable ownership identity for profile-applied assets and provenance.
const PROFILE_OWNER: &str = "profile-application";
/// Stable ownership identity for the worktree `.gitattributes` line-set claim.
const GITATTRIBUTES_OWNER: &str = "gitattributes-merge-driver";

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
    /// The closed semantic seed was invalid.
    #[error(transparent)]
    Seed(#[from] SeedError),
    /// The complete semantic plan could not be hashed.
    #[error(transparent)]
    PlanHash(#[from] PlanHashError),
    /// The finalizer could not compose the audit-log append.
    #[error(transparent)]
    Mutation(#[from] MutationError),
    /// Existing default-rule materialization could not be derived safely.
    #[error("failed to materialize initialization rules: {0}")]
    RuleMaterialization(String),
    /// A profile asset would overwrite an unowned authored occupant.
    #[error(transparent)]
    ProfileTargetConflict(#[from] ProfileTargetConflictError),
    /// Resolved package definitions disagree for one semantic identity.
    #[error(transparent)]
    ContributionComposition(#[from] ContributionCompositionConflict),
    /// A scaffold path is occupied by an unexpected filesystem kind.
    #[error("initialization target '{path:?}' is occupied by an unsupported filesystem kind")]
    UnexpectedOccupant {
        /// The offending path.
        path: VirtualPath,
    },
    #[error("installed profile record '{path:?}' conflicts with package {id}@{version}")]
    InstalledRecordConflict {
        path: VirtualPath,
        id: String,
        version: String,
    },
    #[error("profile metadata path '{0:?}' has unsupported filesystem state")]
    UnsupportedMetadataPath(VirtualPath),
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
    owner: &'static str,
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
        let parsed = crate::declarations::parse_configuration(config.as_bytes())
            .map_err(|error| InitializationError::Config(error.to_string()))?;
        let parsed = parsed.materialization_config(InvariantRegistry::empty());
        let namespaces = namespaces_from_config(&parsed);
        let serialized = serialize_ruleset(&default_ruleset(&namespaces));
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
            project_name,
            gitattributes: GitattributesClaim::NotApplicable,
        })
    }

    /// Attach the boundary-acquired Git-attributes eligibility to this scaffold.
    pub fn with_gitattributes(mut self, claim: GitattributesClaim) -> Self {
        self.gitattributes = claim;
        self
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

    /// The neutral scaffold files as repository-relative `(path, bytes)` pairs.
    ///
    /// The command overlays these onto the captured base so the profile derivation
    /// sees the proposed neutral repository. Excludes the audit log and any profile
    /// contribution.
    pub fn neutral_files(&self) -> Vec<(String, Vec<u8>)> {
        let mut files = vec![
            (".jit/config.toml".to_string(), self.config.clone()),
            (".jit/index.json".to_string(), self.index.clone()),
            (".jit/gates.toml".to_string(), self.gates.clone()),
            (".jit/invariants.toml".to_string(), self.invariants.clone()),
            (".jit/rules.toml".to_string(), self.rules.clone()),
        ];
        files.extend(
            self.schemas
                .iter()
                .map(|(name, bytes)| (format!(".jit/schemas/{name}"), bytes.clone())),
        );
        files
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
                owner: SCAFFOLD_OWNER,
            },
            DesiredFile {
                path: VirtualPath::INDEX,
                bytes: self.index.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: SCAFFOLD_OWNER,
            },
            DesiredFile {
                path: VirtualPath::GATES,
                bytes: self.gates.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: SCAFFOLD_OWNER,
            },
            DesiredFile {
                path: VirtualPath::INVARIANTS,
                bytes: self.invariants.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: SCAFFOLD_OWNER,
            },
            DesiredFile {
                path: VirtualPath::RULES,
                bytes: self.rules.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: SCAFFOLD_OWNER,
            },
        ];
        for (name, bytes) in &self.schemas {
            files.push(DesiredFile {
                path: VirtualPath::data(format!("schemas/{name}"))?,
                bytes: bytes.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::Always,
                owner: SCAFFOLD_OWNER,
            });
        }
        // The audit log is not a neutral scaffold file: the finalizer composes it
        // (an empty log for a plain init, a `ProfileApplied` append for a profiled
        // one) from the captured prefix. See `derive_initialization`.
        Ok(files)
    }

    /// Directories the scaffold must create that hold no delta file. Only the
    /// issues directory qualifies; schema and profile directories are ancestors of
    /// written files and enter the directory closure automatically.
    fn explicit_dirs(&self) -> Result<Vec<VirtualPath>, InitializationError> {
        Ok(vec![VirtualPath::ISSUES])
    }

    /// Every path the composed delta may touch — files, the provenance record, the
    /// explicit directories, and every ancestor directory of those — so the command
    /// discovers them into its capture spec and `apply` finds each action's
    /// captured preimage. Over-inclusion is safe; a missing delta path is not.
    pub fn delta_paths(&self) -> Result<Vec<VirtualPath>, InitializationError> {
        let mut paths: Vec<VirtualPath> = self
            .desired_files()?
            .into_iter()
            .map(|file| file.path)
            .collect();
        // The finalizer composes `events.jsonl` from the captured prefix, so its
        // path must be captured for the prior bytes and the action preimage.
        paths.push(VirtualPath::EVENTS);
        if let Some(profile) = &self.profile {
            paths.push(profile.record_path.clone());
            paths.push(VirtualPath::PROFILES);
            paths.extend(profile.claims.target_paths()?);
            if profile.owns_default_rule_authority() {
                paths.push(VirtualPath::SCHEMAS);
            }
        }
        if self.gitattributes_line().is_some() {
            paths.push(VirtualPath::GITATTRIBUTES);
        }
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
        let Some(profile) = &self.profile else {
            return Ok(Vec::new());
        };
        let proposed = self.profile_composition_base(base)?;
        super::profile_apply::profile_capture_closure(&proposed, profile)
            .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))
    }

    /// Overlay only the neutral scaffold files that initialization will actually
    /// publish, preserving existing `IfAbsent` authored files. This is the
    /// repository view against which profile composition is both captured and
    /// preflighted before the scaffold can publish.
    pub(crate) fn profile_composition_base(
        &self,
        base: &RepositoryImage,
    ) -> Result<RepositoryImage, InitializationError> {
        super::apply_overlay(base, desired_overrides(base, &self.desired_files()?)?)
            .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))
    }

    fn seed(&self) -> Result<RepositorySeed, InitializationError> {
        let facts = BTreeMap::from([(
            "project".to_string(),
            self.project_name.as_str().to_string(),
        )]);
        Ok(RepositorySeed::new(
            RepositorySeedKind::Initialization,
            facts,
            BTreeMap::new(),
        )?)
    }
}

/// Compose the complete initialization delta over the captured base image.
///
/// Every action's expected preimage is derived from the captured base, so
/// `session.apply` revalidates against exactly what was captured. Neutral scaffold
/// files fill missing state (authored content preserved), generated schemas are
/// rewritten, profile provenance writes when it changes, and profile assets write
/// unconditionally (the derivation proved them changed). The audit log is composed
/// by the finalizer (`context`): a plain init creates it empty, a profiled init
/// appends one `ProfileApplied` record. An empty delta (nothing to publish) is a
/// complete no-op.
pub(super) fn derive_initialization(
    base: &RepositoryImage,
    scaffold: &InitializationScaffold,
    context: &MutationContext,
) -> Result<MaterializationDerivation, InitializationError> {
    let mut actions = Vec::new();
    let config_path = VirtualPath::CONFIG;
    let rules_path = VirtualPath::RULES;
    let existing_rules = matches!(base.entry(&rules_path)?, RepositoryEntry::File { .. });
    let profile_changes_rules_authority = scaffold
        .profile
        .as_ref()
        .is_some_and(ProfileApplicationInput::owns_default_rule_authority);
    let compose_rules = existing_rules || profile_changes_rules_authority;
    let generated_paths: std::collections::BTreeSet<VirtualPath> = scaffold
        .schemas
        .iter()
        .map(|(name, _)| VirtualPath::data(format!("schemas/{name}")))
        .collect::<Result<_, _>>()?;
    let mut desired = scaffold.desired_files()?;
    let neutral_proposed = super::apply_overlay(base, desired_overrides(base, &desired)?)
        .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))?;
    let mut profile_targets = Vec::new();
    let mut profile_changed = false;
    let mut profile_record = None;
    if let Some(profile) = &scaffold.profile {
        let composition_base =
            super::profile_apply::profile_composition_base(&neutral_proposed, profile)
                .map_err(profile_composition_error)?;
        let composed = super::profile_apply::compose_profile_targets_with_context(
            &composition_base,
            profile.claims.clone(),
            profile.contribution_context.clone(),
        )
        .map_err(profile_composition_error)?;
        let record = profile
            .record(&composed)
            .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))?;
        for (path, (bytes, mode)) in composed.targets {
            let disposition = profile_target_disposition(&neutral_proposed, &path, &bytes, mode)?;
            profile_targets.push(ProfileTargetMaterialization {
                path: path.clone(),
                disposition,
                mode,
            });
            if disposition != ProfileTargetDisposition::Unchanged {
                profile_changed = true;
                desired.push(DesiredFile {
                    path,
                    bytes,
                    mode,
                    policy: WritePolicy::Always,
                    owner: PROFILE_OWNER,
                });
            }
        }
        profile_changed |= profile_record_changed(base, profile, &record)?;
        profile_changed |= !profile.shipped_v1_migrations.is_empty();
        profile_record = Some(record);
    }
    let desired = dedup_last_wins(desired);
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
    if let (Some(profile), Some(record)) = (&scaffold.profile, &profile_record) {
        let migrated_records = profile
            .shipped_v1_migrations
            .iter()
            .filter(|(path, _)| *path != &profile.record_path)
            .map(|(path, record)| {
                Ok(DesiredFile {
                    path: path.clone(),
                    bytes: serialize_profile_record(record)?,
                    mode: FileMode::Regular,
                    policy: WritePolicy::Always,
                    owner: PROFILE_OWNER,
                })
            })
            .collect::<Result<Vec<_>, InitializationError>>()?;
        push_file_actions(base, &migrated_records, &mut actions)?;
        push_record_action(base, profile, record, profile_changed, &mut actions)?;
    }
    if let Some(action) = events_action(base, scaffold.profile.as_ref(), profile_changed, context)?
    {
        actions.push(action);
    }
    push_gitattributes_action(base, scaffold, &mut actions)?;
    let mut all = directory_actions(base, &actions, &scaffold.explicit_dirs()?)?;
    all.extend(actions);
    let delta = RepositoryDelta::new(base.layout(), all)?;
    Ok(MaterializationDerivation::new(
        delta,
        scaffold.seed()?,
        MaterializationIntent::InitializeRepository,
    )
    .with_profile_targets(profile_targets))
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

fn profile_target_disposition(
    base: &RepositoryImage,
    path: &VirtualPath,
    bytes: &[u8],
    mode: FileMode,
) -> Result<ProfileTargetDisposition, InitializationError> {
    Ok(match base.entry(path)? {
        RepositoryEntry::Absent => ProfileTargetDisposition::Create,
        RepositoryEntry::File {
            bytes: existing,
            mode: existing_mode,
            ..
        } if existing.as_slice() == bytes && *existing_mode == mode => {
            ProfileTargetDisposition::Unchanged
        }
        _ => ProfileTargetDisposition::Update,
    })
}

fn profile_record_changed(
    base: &RepositoryImage,
    profile: &ProfileApplicationInput,
    record: &AppliedProfileRecord,
) -> Result<bool, InitializationError> {
    // The selected package may replace the same shipped-v1 record that the
    // operation authenticated. Its raw v1 preimage is still carried by this
    // final record write, while the current selected record is the one final
    // persisted provenance image.
    if profile
        .shipped_v1_migrations
        .contains_key(&profile.record_path)
    {
        return Ok(true);
    }
    match base.entry(&profile.record_path)? {
        RepositoryEntry::Absent => Ok(true),
        RepositoryEntry::File { bytes, .. } => {
            let existing = serde_json::from_slice::<AppliedProfileRecord>(bytes);
            match existing {
                Ok(existing) if existing == *record => Ok(false),
                Ok(existing) if existing.matches_package_provenance(record) => Ok(true),
                _ => Err(InitializationError::InstalledRecordConflict {
                    path: profile.record_path.clone(),
                    id: profile.id.to_string(),
                    version: profile.version.clone(),
                }),
            }
        }
        _ => Err(InitializationError::UnsupportedMetadataPath(
            profile.record_path.clone(),
        )),
    }
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
    action: RepositoryAction,
) -> Result<RepositoryAction, InitializationError> {
    let expected = ExpectedPreimage::of(base.entry(action.path())?);
    Ok(match action {
        RepositoryAction::CreateDirectory { path, owner, .. } => {
            RepositoryAction::CreateDirectory {
                path,
                owner,
                expected,
            }
        }
        RepositoryAction::WriteFile {
            path,
            owner,
            bytes,
            mode,
            ..
        } => RepositoryAction::WriteFile {
            path,
            owner,
            expected,
            bytes,
            mode,
        },
        RepositoryAction::SetMode {
            path, owner, mode, ..
        } => RepositoryAction::SetMode {
            path,
            owner,
            expected,
            mode,
        },
        RepositoryAction::DeleteFile { path, owner, .. } => RepositoryAction::DeleteFile {
            path,
            owner,
            expected,
        },
    })
}

/// Compose the `events.jsonl` action for an init/profile delta.
///
/// A profile whose application emits an event appends one `ProfileApplied` record
/// through the finalizer — [`finalize_audit_append`] assigns its id/timestamp and
/// torn-tail evidence over the captured prefix, so command code never computes
/// audit-log bytes. Otherwise the log is created empty when absent (fresh init) and
/// left untouched when already present.
fn events_action(
    base: &RepositoryImage,
    profile: Option<&ProfileApplicationInput>,
    profile_changed: bool,
    context: &MutationContext,
) -> Result<Option<RepositoryAction>, InitializationError> {
    if let Some(profile) = profile {
        if profile_changed {
            return profile_event_action(base, profile, context);
        }
    }
    let path = VirtualPath::EVENTS;
    match base.entry(&path)? {
        RepositoryEntry::Absent => Ok(Some(RepositoryAction::WriteFile {
            path,
            owner: SCAFFOLD_OWNER.to_string(),
            expected: ExpectedPreimage::Absent,
            bytes: Vec::new(),
            mode: FileMode::Regular,
        })),
        RepositoryEntry::File { .. } => Ok(None),
        _ => Err(InitializationError::UnexpectedOccupant { path }),
    }
}

/// Compose one profile-applied audit append over the captured event prefix.
fn profile_event_action(
    base: &RepositoryImage,
    profile: &ProfileApplicationInput,
    context: &MutationContext,
) -> Result<Option<RepositoryAction>, InitializationError> {
    let event = profile_applied_event(
        profile.id.to_string(),
        profile.version.clone(),
        profile.origin.clone(),
        profile.package_hash.clone(),
        profile.target_hashes.clone(),
    );
    Ok(finalize_audit_append(base, context, vec![(2, event)])?)
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
            owner: GITATTRIBUTES_OWNER.to_string(),
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
/// any journaling.
fn resolve_gitattributes(
    entry: &RepositoryEntry,
    line: &str,
) -> Result<(GitattributesStatus, Option<Vec<u8>>), InitializationError> {
    let block = format!("{GITATTRIBUTES_MARKER}\n{line}\n");
    match entry {
        RepositoryEntry::Absent => Ok((GitattributesStatus::Created, Some(block.into_bytes()))),
        RepositoryEntry::File { bytes, .. } => {
            let content = std::str::from_utf8(bytes).map_err(|_| {
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

/// Compose the complete profile-application delta over an existing repository's
/// captured base image (no neutral scaffold).
pub(super) fn derive_profile_application(
    base: &RepositoryImage,
    profile: &ProfileApplicationInput,
    context: &MutationContext,
) -> Result<MaterializationDerivation, InitializationError> {
    let composition_base = super::profile_apply::profile_composition_base(base, profile)
        .map_err(profile_composition_error)?;
    let composed = super::profile_apply::compose_profile_targets_with_context(
        &composition_base,
        profile.claims.clone(),
        profile.contribution_context.clone(),
    )
    .map_err(profile_composition_error)?;
    let record = profile
        .record(&composed)
        .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))?;
    let mut targets = Vec::with_capacity(composed.targets.len());
    let files = composed
        .targets
        .into_iter()
        .map(|(path, (bytes, mode))| {
            let disposition = profile_target_disposition(base, &path, &bytes, mode)?;
            targets.push(ProfileTargetMaterialization {
                path: path.clone(),
                disposition,
                mode,
            });
            Ok(
                (disposition != ProfileTargetDisposition::Unchanged).then_some(DesiredFile {
                    path,
                    bytes,
                    mode,
                    policy: WritePolicy::Always,
                    owner: PROFILE_OWNER,
                }),
            )
        })
        .collect::<Result<Vec<_>, InitializationError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let mut actions = Vec::new();
    push_file_actions(base, &files, &mut actions)?;
    let migrated_records = profile
        .shipped_v1_migrations
        .iter()
        .filter(|(path, _)| *path != &profile.record_path)
        .map(|(path, record)| {
            Ok(DesiredFile {
                path: path.clone(),
                bytes: serialize_profile_record(record)?,
                mode: FileMode::Regular,
                policy: WritePolicy::Always,
                owner: PROFILE_OWNER,
            })
        })
        .collect::<Result<Vec<_>, InitializationError>>()?;
    push_file_actions(base, &migrated_records, &mut actions)?;
    let config_path = VirtualPath::CONFIG;
    let rules_path = VirtualPath::RULES;
    if profile.target_hashes.contains_key(".jit/config.toml")
        || profile.target_hashes.contains_key(".jit/rules.toml")
    {
        let authority = authored_rule_overrides(base, &files, &config_path, &rules_path)?;
        compose_existing_default_rules(base, authority, &mut actions)?;
    }
    match base.entry(&profile.record_path)? {
        RepositoryEntry::Absent => actions.push(RepositoryAction::WriteFile {
            path: profile.record_path.clone(),
            owner: PROFILE_OWNER.to_string(),
            expected: ExpectedPreimage::Absent,
            bytes: serialize_profile_record(&record)?,
            mode: FileMode::Regular,
        }),
        RepositoryEntry::File { .. } => {
            if profile_record_changed(base, profile, &record)? {
                push_file_actions(
                    base,
                    &[DesiredFile {
                        path: profile.record_path.clone(),
                        bytes: serialize_profile_record(&record)?,
                        mode: FileMode::Regular,
                        policy: WritePolicy::Always,
                        owner: PROFILE_OWNER,
                    }],
                    &mut actions,
                )?;
            }
        }
        _ => {
            return Err(InitializationError::UnsupportedMetadataPath(
                profile.record_path.clone(),
            ))
        }
    }
    if !actions.is_empty() {
        let event = profile_applied_event(
            profile.id.to_string(),
            profile.version.clone(),
            profile.origin.clone(),
            profile.package_hash.clone(),
            profile.target_hashes.clone(),
        );
        if let Some(action) = finalize_audit_append(base, context, vec![(2, event)])? {
            actions.push(action);
        }
    }
    let profiles = VirtualPath::PROFILES;
    let explicit = match base.entry(&profiles)? {
        RepositoryEntry::Absent => vec![profiles],
        RepositoryEntry::Directory { .. } => Vec::new(),
        _ => return Err(InitializationError::UnsupportedMetadataPath(profiles)),
    };
    let mut all = directory_actions(base, &actions, &explicit)?;
    all.extend(actions);
    let delta = RepositoryDelta::new(base.layout(), all)?;
    let facts = BTreeMap::from([("profile".to_string(), profile.id.to_string())]);
    let seed = RepositorySeed::new(
        RepositorySeedKind::Profile {
            name: profile.id.to_string(),
            version: profile.version.clone(),
            package_hash: profile.package_hash.clone(),
        },
        facts,
        BTreeMap::new(),
    )?;
    Ok(
        MaterializationDerivation::new(delta, seed, MaterializationIntent::ApplyProfile)
            .with_profile_targets(targets),
    )
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

/// Emit the provenance-record write only when the command proved it changed.
fn push_record_action(
    base: &RepositoryImage,
    profile: &ProfileApplicationInput,
    record: &AppliedProfileRecord,
    changed: bool,
    actions: &mut Vec<RepositoryAction>,
) -> Result<(), InitializationError> {
    if changed && profile_record_changed(base, profile, record)? {
        push_file_actions(
            base,
            &[DesiredFile {
                path: profile.record_path.clone(),
                bytes: serialize_profile_record(record)?,
                mode: FileMode::Regular,
                policy: WritePolicy::Always,
                owner: PROFILE_OWNER,
            }],
            actions,
        )?;
    }
    Ok(())
}

fn serialize_profile_record(record: &AppliedProfileRecord) -> Result<Vec<u8>, InitializationError> {
    record
        .to_bytes()
        .map_err(|error| InitializationError::RuleMaterialization(error.to_string()))
}

fn profile_composition_error(error: RepositoryStateError) -> InitializationError {
    match error {
        RepositoryStateError::ProfileTargetConflict(error) => error.into(),
        RepositoryStateError::ContributionComposition(error) => error.into(),
        error => InitializationError::RuleMaterialization(error.to_string()),
    }
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
                owner: SCAFFOLD_OWNER.to_string(),
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
                owner: file.owner.to_string(),
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
    use crate::repository_state::EntryIdentity;

    const LINE: &str = ".jit/events.jsonl merge=union";

    fn file(bytes: &[u8]) -> RepositoryEntry {
        RepositoryEntry::File {
            identity: EntryIdentity::for_bytes("obj", bytes).unwrap(),
            bytes: bytes.to_vec(),
            mode: FileMode::Regular,
        }
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
