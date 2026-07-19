//! Repository initialization and profile-application materialization producers.
//!
//! `repository_state` owns fresh-config rendering and the complete init/profile
//! delta. A command submits semantic choices — the template config skeleton, the
//! project name, and (for a profile) the embedded package's asset bytes,
//! provenance record, and audit-log bytes — and this module renders every neutral
//! byte and composes one exact [`RepositoryDelta`] over the captured base image.
//! The command opens the recovered session, captures the base under its guard,
//! and applies the returned plan; it never hand-builds a transaction or a
//! command-local final-byte inventory.
//!
//! Two dispositions are scheduled predecessor debt carried into this delta rather
//! than rebuilt here: the profile's asset bytes still arrive from the command's
//! `plan_profile_application_against` planner (its `repository_state::TargetClaim`
//! replacement lands in increment 6), and the `ProfileApplied` audit line still
//! arrives as command-computed bytes (its finalizer-routed replacement lands in
//! increment 7). Both flow through `session.apply` here; no new call site is added
//! to those predecessors.

use std::collections::BTreeMap;

use crate::config::{JitConfig, ProjectName};
use crate::config_manager::namespaces_from_config;
use crate::declarations::{serialize_gate_registry, GateRegistry};

use super::default_rules::default_ruleset;
use super::image::{
    CaptureError, DeltaError, ExpectedPreimage, FileMode, MaterializationIntent, PlanHashError,
    RepositoryAction, RepositoryDelta, RepositoryEntry, RepositoryImage, RepositorySeed,
    RepositorySeedKind, SeedError,
};
use super::mutation::fresh_index_bytes;
use super::path::{RepositoryLayoutError, RootRelativePath, VirtualPath};
use super::rule_serialize::serialize_ruleset;
use super::MaterializationPlan;

/// Stable ownership identity for the neutral scaffold materialization.
const SCAFFOLD_OWNER: &str = "repository-init";
/// Stable ownership identity for profile-applied assets and provenance.
const PROFILE_OWNER: &str = "profile-application";

/// Render the canonical fresh `config.toml` from a template body plus identity.
///
/// The exact scaffold format: template body, then the project-identity table with
/// its guiding comment. Kept byte-for-byte to preserve the generated file a user
/// first sees after `jit init`. `repository_state` owns this rendering so init
/// submits the template choice and identity, never final bytes.
pub fn render_repo_config(base_config_toml: &str, project_name: &ProjectName) -> String {
    format!(
        "{}\n# =============================================================================\n# PROJECT IDENTITY\n# =============================================================================\n# Canonical, human-editable project name: the `@<project>` scope token in the\n# multi-jit addressing scheme. Must match ^[a-z][a-z0-9-]*$. Defaults to a\n# slug of this repository's directory name; edit freely.\n[project]\nname = \"{}\"\n",
        base_config_toml,
        project_name.as_str()
    )
}

/// One profile asset target carried into the init/profile delta.
///
/// The bytes originate from the command's profile planner (increment-6 deletion
/// target); this type simply carries them to the producer as a canonical claim.
#[derive(Debug, Clone)]
pub struct ProfileTargetContribution {
    /// Canonical target path.
    pub path: VirtualPath,
    /// Exact final bytes.
    pub bytes: Vec<u8>,
    /// Exact final mode.
    pub mode: FileMode,
}

/// The complete profile contribution merged into an init or apply delta.
#[derive(Debug, Clone)]
pub struct ProfileContribution {
    /// Profile identity.
    pub id: String,
    /// Profile declaration version.
    pub version: String,
    /// Exact package identity.
    pub package_hash: String,
    /// Non-noop asset targets (the command's planner pre-filters unchanged ones).
    pub targets: Vec<ProfileTargetContribution>,
    /// Canonical applied-provenance record path (`.jit/profiles/<id>.json`).
    pub record_path: VirtualPath,
    /// Exact provenance record bytes.
    pub record_bytes: Vec<u8>,
    /// Whether the captured provenance record differs from `record_bytes`.
    pub record_changed: bool,
    /// Exact audit-log bytes (captured prefix plus the appended `ProfileApplied`).
    pub events_bytes: Vec<u8>,
    /// Whether the audit log changed from its captured prefix.
    pub events_changed: bool,
    /// Whether the `.jit/profiles` directory must be created.
    pub ensure_profiles_dir: bool,
}

impl ProfileContribution {
    /// Every path a standalone profile-application delta may touch — asset targets,
    /// the provenance record, the audit log, the profiles directory, and every
    /// ancestor directory — so the command discovers them into its capture spec.
    pub fn delta_paths(&self) -> Result<Vec<VirtualPath>, InitializationError> {
        let mut paths: Vec<VirtualPath> = self
            .targets
            .iter()
            .map(|target| target.path.clone())
            .collect();
        paths.push(self.record_path.clone());
        paths.push(VirtualPath::data("events.jsonl")?);
        if self.ensure_profiles_dir {
            paths.push(VirtualPath::data("profiles")?);
        }
        with_ancestor_dirs(paths)
    }

    /// The proposed bytes for every profile target, for the command's capture
    /// closure and proposed-state validation overlay.
    pub fn overlay_overrides(
        &self,
    ) -> Result<BTreeMap<VirtualPath, Option<Vec<u8>>>, InitializationError> {
        let mut overrides: BTreeMap<VirtualPath, Option<Vec<u8>>> = self
            .targets
            .iter()
            .map(|target| (target.path.clone(), Some(target.bytes.clone())))
            .collect();
        overrides.insert(
            VirtualPath::data("events.jsonl")?,
            Some(self.events_bytes.clone()),
        );
        if self.record_changed {
            overrides.insert(self.record_path.clone(), Some(self.record_bytes.clone()));
        }
        Ok(overrides)
    }
}

/// Failure while rendering or composing an initialization/profile delta.
#[derive(Debug, thiserror::Error)]
pub enum InitializationError {
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
    /// A scaffold path is occupied by an unexpected filesystem kind.
    #[error("initialization target '{path}' is occupied by an unsupported filesystem kind")]
    UnexpectedOccupant {
        /// The offending path.
        path: String,
    },
}

/// When a desired file is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WritePolicy {
    /// Write only when the base captured the path as absent (preserve authored
    /// content; fill missing scaffold state).
    IfAbsent,
    /// Always write (repair-owned generated content; profile assets the planner
    /// already proved changed).
    Always,
    /// Write only when the captured bytes differ from the desired bytes.
    IfChanged,
}

/// One desired file target with its write policy and ownership.
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
    rules: Vec<u8>,
    schemas: Vec<(String, Vec<u8>)>,
    profile: Option<ProfileContribution>,
    project_name: ProjectName,
}

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
        profile: Option<ProfileContribution>,
    ) -> Result<Self, InitializationError> {
        let config = render_repo_config(config_skeleton, &project_name);
        Self::from_config(config, project_name, profile)
    }

    /// Render a scaffold whose configuration bytes are already final (an existing
    /// repository's captured `config.toml`), deriving default rules/schemas from it.
    pub fn from_config(
        config: String,
        project_name: ProjectName,
        profile: Option<ProfileContribution>,
    ) -> Result<Self, InitializationError> {
        let parsed: JitConfig = toml::from_str(&config)
            .map_err(|error| InitializationError::Config(error.to_string()))?;
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
            rules: serialized.rules_toml.into_bytes(),
            schemas,
            profile,
            project_name,
        })
    }

    /// The canonical project identity this scaffold publishes.
    pub fn project_name(&self) -> &ProjectName {
        &self.project_name
    }

    /// The neutral scaffold files as repository-relative `(path, bytes)` pairs.
    ///
    /// The command overlays these onto its transitional profile-planning view (the
    /// `RepositoryView`/planner path that increment 6 deletes) so profile
    /// application sees the proposed neutral repository. Excludes the audit log and
    /// any profile contribution.
    pub fn neutral_files(&self) -> Vec<(String, Vec<u8>)> {
        let mut files = vec![
            (".jit/config.toml".to_string(), self.config.clone()),
            (".jit/index.json".to_string(), self.index.clone()),
            (".jit/gates.toml".to_string(), self.gates.clone()),
            (".jit/rules.toml".to_string(), self.rules.clone()),
        ];
        files.extend(
            self.schemas
                .iter()
                .map(|(name, bytes)| (format!(".jit/schemas/{name}"), bytes.clone())),
        );
        files
    }

    /// Whether the scaffold carries an embedded profile contribution.
    pub fn profile(&self) -> Option<&ProfileContribution> {
        self.profile.as_ref()
    }

    /// Every desired neutral (and profile) file, with its write policy.
    fn desired_files(&self) -> Result<Vec<DesiredFile>, InitializationError> {
        let mut files = vec![
            DesiredFile {
                path: VirtualPath::data("config.toml")?,
                bytes: self.config.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: SCAFFOLD_OWNER,
            },
            DesiredFile {
                path: VirtualPath::data("index.json")?,
                bytes: self.index.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: SCAFFOLD_OWNER,
            },
            DesiredFile {
                path: VirtualPath::data("gates.toml")?,
                bytes: self.gates.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::IfAbsent,
                owner: SCAFFOLD_OWNER,
            },
            DesiredFile {
                path: VirtualPath::data("rules.toml")?,
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
        // The audit log is created empty for a plain init and carries the profile
        // event image for a profiled init; write it whenever it differs from the
        // captured prefix (absent for fresh init).
        files.push(DesiredFile {
            path: VirtualPath::data("events.jsonl")?,
            bytes: self
                .profile
                .as_ref()
                .map_or_else(Vec::new, |profile| profile.events_bytes.clone()),
            mode: FileMode::Regular,
            policy: WritePolicy::IfChanged,
            owner: SCAFFOLD_OWNER,
        });
        if let Some(profile) = &self.profile {
            files.extend(profile_asset_files(profile));
        }
        Ok(dedup_last_wins(files))
    }

    /// Directories the scaffold must create that hold no delta file. Only the
    /// issues directory qualifies; schema and profile directories are ancestors of
    /// written files and enter the directory closure automatically.
    fn explicit_dirs(&self) -> Result<Vec<VirtualPath>, InitializationError> {
        Ok(vec![VirtualPath::data("issues")?])
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
        if let Some(profile) = &self.profile {
            paths.push(profile.record_path.clone());
        }
        paths.extend(self.explicit_dirs()?);
        with_ancestor_dirs(paths)
    }

    /// The proposed bytes for every scaffold and profile target, for the command's
    /// whole-repository proposed-state validation overlay.
    pub fn overlay_overrides(
        &self,
    ) -> Result<BTreeMap<VirtualPath, Option<Vec<u8>>>, InitializationError> {
        let mut overrides: BTreeMap<VirtualPath, Option<Vec<u8>>> = self
            .desired_files()?
            .into_iter()
            .map(|file| (file.path, Some(file.bytes)))
            .collect();
        if let Some(profile) = &self.profile {
            if profile.record_changed {
                overrides.insert(
                    profile.record_path.clone(),
                    Some(profile.record_bytes.clone()),
                );
            }
        }
        Ok(overrides)
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
/// rewritten, the audit log and profile provenance write when they change, and
/// profile assets write unconditionally (the planner proved them changed). An
/// empty delta (nothing to publish) is a complete no-op.
pub fn finalize_initialization(
    base: &RepositoryImage,
    scaffold: &InitializationScaffold,
) -> Result<MaterializationPlan, InitializationError> {
    let mut actions = Vec::new();
    push_file_actions(base, &scaffold.desired_files()?, &mut actions)?;
    if let Some(profile) = &scaffold.profile {
        push_record_action(base, profile, &mut actions)?;
    }
    let mut all = directory_actions(base, &actions, &scaffold.explicit_dirs()?)?;
    all.extend(actions);
    let delta = RepositoryDelta::new(base.layout(), all)?;
    let seed = scaffold.seed()?;
    Ok(MaterializationPlan::new(
        base,
        &seed,
        &MaterializationIntent::InitializeRepository,
        delta,
    )?)
}

/// Compose the complete profile-application delta over an existing repository's
/// captured base image (no neutral scaffold).
pub fn finalize_profile_application(
    base: &RepositoryImage,
    profile: &ProfileContribution,
) -> Result<MaterializationPlan, InitializationError> {
    let mut files = profile_asset_files(profile);
    files.push(DesiredFile {
        path: VirtualPath::data("events.jsonl")?,
        bytes: profile.events_bytes.clone(),
        mode: FileMode::Regular,
        policy: WritePolicy::IfChanged,
        owner: PROFILE_OWNER,
    });
    let mut actions = Vec::new();
    push_file_actions(base, &files, &mut actions)?;
    push_record_action(base, profile, &mut actions)?;
    let explicit = if profile.ensure_profiles_dir {
        vec![VirtualPath::data("profiles")?]
    } else {
        Vec::new()
    };
    let mut all = directory_actions(base, &actions, &explicit)?;
    all.extend(actions);
    let delta = RepositoryDelta::new(base.layout(), all)?;
    let facts = BTreeMap::from([("profile".to_string(), profile.id.clone())]);
    let seed = RepositorySeed::new(
        RepositorySeedKind::Profile {
            name: profile.id.clone(),
            version: profile.version.clone(),
            package_hash: profile.package_hash.clone(),
        },
        facts,
        BTreeMap::new(),
    )?;
    Ok(MaterializationPlan::new(
        base,
        &seed,
        &MaterializationIntent::ApplyProfile,
        delta,
    )?)
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

/// The profile's asset targets (the planner pre-filtered unchanged ones). The
/// audit log and provenance record are handled by their own policies at the call
/// sites, so they are not included here.
fn profile_asset_files(profile: &ProfileContribution) -> Vec<DesiredFile> {
    profile
        .targets
        .iter()
        .map(|target| DesiredFile {
            path: target.path.clone(),
            bytes: target.bytes.clone(),
            mode: target.mode,
            policy: WritePolicy::Always,
            owner: PROFILE_OWNER,
        })
        .collect()
}

/// Emit the provenance-record write only when the command proved it changed.
fn push_record_action(
    base: &RepositoryImage,
    profile: &ProfileContribution,
    actions: &mut Vec<RepositoryAction>,
) -> Result<(), InitializationError> {
    if profile.record_changed {
        push_file_actions(
            base,
            &[DesiredFile {
                path: profile.record_path.clone(),
                bytes: profile.record_bytes.clone(),
                mode: FileMode::Regular,
                policy: WritePolicy::Always,
                owner: PROFILE_OWNER,
            }],
            actions,
        )?;
    }
    Ok(())
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
            _ => {
                return Err(InitializationError::UnexpectedOccupant {
                    path: format!("{dir:?}"),
                })
            }
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
                    path: format!("{:?}", file.path),
                })
            }
        }
        let write = match file.policy {
            WritePolicy::IfAbsent => matches!(entry, RepositoryEntry::Absent),
            WritePolicy::Always => true,
            WritePolicy::IfChanged => !file_matches(entry, &file.bytes, file.mode),
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

/// Whether a captured entry is a file already holding exactly `bytes` and `mode`.
fn file_matches(entry: &RepositoryEntry, bytes: &[u8], mode: FileMode) -> bool {
    matches!(
        entry,
        RepositoryEntry::File {
            bytes: captured,
            mode: captured_mode,
            ..
        } if captured.as_slice() == bytes && *captured_mode == mode
    )
}
