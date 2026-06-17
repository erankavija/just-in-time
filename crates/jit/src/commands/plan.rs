//! Bracket scaffolding command (design doc T5 / D10).
//!
//! Brackets a breakable container `C` with a planning node `P`, sequenced
//! *before* the implementation fan-out. At scaffold only `C` and `P` exist:
//!
//! ```text
//! Scaffold:    C ──dep→ P
//! ```
//!
//! Two entry points (D10):
//!
//! - [`plan_existing`](CommandExecutor::plan_existing) — RETROFIT an existing
//!   container. `C`'s pre-existing upstream dependencies MOVE onto `P` (the front
//!   of the spine), so planning waits on the upstream work and `C` becomes the
//!   pure closure node at the back of its own bracket.
//! - [`create_with_planning`](CommandExecutor::create_with_planning) — create a
//!   brand-new container AND its planning node in one shot.
//!
//! Both create `P` typed `type:<planning_type>`, wire `C → P`, apply the
//! `plan_gate_preset` (the agent plan-quality gate) to `P`, and set `P`'s
//! plan-doc location from the `[planning]` config (T4 resolver). Neither creates
//! the breakdown node `B` — that is the breakdown step's job (T10).
//!
//! # Domain-agnostic
//!
//! No `epic` / `planning` / `breakdown` literal is hardcoded: every type name,
//! the gate preset name, and the plan-doc template come from
//! [`PlanningConfig`](crate::config::PlanningConfig), so an adopting ruleset can
//! declare an entirely different vocabulary (SDD `epic`, research `goal`, ...).

use super::*;
use crate::commands::plan_doc::{resolve_plan_doc_location, PlanDocLocation};
use crate::config::PlanningConfig;
use crate::domain::{ContentFormat, DocumentReference};
use serde::Serialize;

/// Outcome of a bracket-scaffolding operation.
///
/// Names the container and the planning node it was bracketed with, the
/// resolved planning type, the gate preset applied to `P`, and the resolved
/// plan-doc location (the literal `"inline"` or an `{id}`-substituted path). Used
/// for `--json` output and as the in-process return value.
///
/// # Examples
///
/// ```
/// use jit::commands::PlanResult;
///
/// let result = PlanResult {
///     container_id: "c1".to_string(),
///     planning_id: "p1".to_string(),
///     planning_type: "planning".to_string(),
///     plan_gate_preset: "plan-review".to_string(),
///     plan_doc_location: "inline".to_string(),
/// };
/// assert_eq!(result.planning_type, "planning");
/// ```
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PlanResult {
    /// The bracketed container `C`.
    pub container_id: String,
    /// The planning node `P` created in front of `C`.
    pub planning_id: String,
    /// The type name carried by `P` (from `[planning].planning_type`).
    pub planning_type: String,
    /// The gate preset applied to `P` (from `[planning].plan_gate_preset`).
    pub plan_gate_preset: String,
    /// The resolved plan-doc location for `P`: the literal `"inline"` or an
    /// `{id}`-substituted external path.
    pub plan_doc_location: String,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Resolve the effective `[planning]` configuration, erroring clearly when
    /// the repository has not declared a bracket vocabulary.
    fn planning_config(&self) -> Result<PlanningConfig> {
        self.cached_config()?.planning.clone().ok_or_else(|| {
            anyhow!(
                "no [planning] section in .jit/config.toml: planning brackets require a \
                 declared vocabulary (breakable_types, planning_type, plan_gate_preset, ...)"
            )
        })
    }

    /// Retrofit an existing container `C` into a planning bracket (`jit plan <id>`).
    ///
    /// Reads the `[planning]` vocabulary from `.jit/config.toml` and delegates to
    /// [`plan_existing_with_config`](Self::plan_existing_with_config). See that
    /// method for the full wiring contract (create `P`, move `C`'s upstream deps
    /// onto `P`, wire `C → P`, apply the plan-review preset, set `P`'s plan-doc
    /// location; never creates the breakdown node).
    ///
    /// Returns the [`PlanResult`] plus any lease warnings collected from the
    /// underlying mutating operations.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let (result, _warnings) = executor.plan_existing("epic-123", false).unwrap();
    /// println!("bracketed {} with {}", result.container_id, result.planning_id);
    /// ```
    pub fn plan_existing(
        &self,
        container_id: &str,
        force: bool,
    ) -> Result<(PlanResult, Vec<String>)> {
        let config = self.planning_config()?;
        self.plan_existing_with_config(&config, container_id, force)
    }

    /// Retrofit an existing container `C` using an explicit `[planning]` config.
    ///
    /// The config-injecting core of [`plan_existing`](Self::plan_existing),
    /// separated so the bracket logic is testable without an on-disk
    /// `config.toml`. Steps:
    ///
    /// 1. Validate `C`'s `type:` label is one of `config.breakable_types`
    ///    (clear error otherwise).
    /// 2. Create `P` typed `type:<planning_type>`.
    /// 3. MOVE each pre-existing upstream dependency `U` of `C` onto `P`
    ///    (`P` depends on `U`), then make `C`'s only dependency `P` (`C → P`).
    /// 4. Apply the `plan_gate_preset` to `P`.
    /// 5. Set `P`'s plan-doc location from `config.plan_doc_location`.
    ///
    /// It does NOT create the breakdown node `B`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::config::PlanningConfig;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let config = PlanningConfig {
    ///     breakable_types: vec!["epic".into()],
    ///     planning_type: "planning".into(),
    ///     breakdown_type: "breakdown".into(),
    ///     plan_doc_location: "inline".into(),
    ///     plan_gate_preset: "plan-review".into(),
    ///     coverage_gate_preset: "coverage-preview".into(),
    /// };
    /// let (result, _warnings) =
    ///     executor.plan_existing_with_config(&config, "epic-123", false).unwrap();
    /// assert_eq!(result.planning_type, "planning");
    /// ```
    pub fn plan_existing_with_config(
        &self,
        config: &PlanningConfig,
        container_id: &str,
        force: bool,
    ) -> Result<(PlanResult, Vec<String>)> {
        let full_container_id = self.storage.resolve_issue_id(container_id)?;
        let container = self.storage.load_issue(&full_container_id)?;
        self.ensure_breakable(config, &container)?;
        self.bracket_container(config, &full_container_id, force)
    }

    /// Create a brand-new container AND its planning bracket in one shot
    /// (`jit issue create --with-planning`).
    ///
    /// Reads the `[planning]` vocabulary from `.jit/config.toml` and delegates to
    /// [`create_with_planning_with_config`](Self::create_with_planning_with_config).
    ///
    /// Returns the [`PlanResult`] plus any warnings (issue-creation warnings
    /// followed by lease warnings).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::domain::Priority;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let (result, _warnings) = executor
    ///     .create_with_planning(
    ///         "New epic".to_string(),
    ///         String::new(),
    ///         Priority::Normal,
    ///         vec![],
    ///         vec!["type:epic".to_string()],
    ///         None,
    ///         false,
    ///     )
    ///     .unwrap();
    /// println!("created {} bracketed by {}", result.container_id, result.planning_id);
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn create_with_planning(
        &self,
        title: String,
        description: String,
        priority: Priority,
        gates: Vec<String>,
        labels: Vec<String>,
        content_format: Option<ContentFormat>,
        force: bool,
    ) -> Result<(PlanResult, Vec<String>)> {
        let config = self.planning_config()?;
        self.create_with_planning_with_config(
            &config,
            title,
            description,
            priority,
            gates,
            labels,
            content_format,
            force,
        )
    }

    /// Create a brand-new container and bracket it, using an explicit config.
    ///
    /// The config-injecting core of
    /// [`create_with_planning`](Self::create_with_planning). Validates the
    /// requested `type:` label is breakable BEFORE creating anything (so a
    /// non-breakable request leaves no orphaned container), creates `C`, then
    /// runs the same bracket wiring as the retrofit path — except a brand-new
    /// container has no pre-existing upstream deps to move, so `P` is simply
    /// wired in front of `C` (`C → P`). Never creates the breakdown node `B`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::commands::CommandExecutor;
    /// use jit::config::PlanningConfig;
    /// use jit::domain::Priority;
    /// use jit::storage::JsonFileStorage;
    ///
    /// let executor = CommandExecutor::new(JsonFileStorage::new(".jit"));
    /// let config = PlanningConfig {
    ///     breakable_types: vec!["epic".into()],
    ///     planning_type: "planning".into(),
    ///     breakdown_type: "breakdown".into(),
    ///     plan_doc_location: "inline".into(),
    ///     plan_gate_preset: "plan-review".into(),
    ///     coverage_gate_preset: "coverage-preview".into(),
    /// };
    /// let (result, _warnings) = executor
    ///     .create_with_planning_with_config(
    ///         &config,
    ///         "New epic".to_string(),
    ///         String::new(),
    ///         Priority::Normal,
    ///         vec![],
    ///         vec!["type:epic".to_string()],
    ///         None,
    ///         false,
    ///     )
    ///     .unwrap();
    /// assert_eq!(result.planning_type, "planning");
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn create_with_planning_with_config(
        &self,
        config: &PlanningConfig,
        title: String,
        description: String,
        priority: Priority,
        gates: Vec<String>,
        labels: Vec<String>,
        content_format: Option<ContentFormat>,
        force: bool,
    ) -> Result<(PlanResult, Vec<String>)> {
        // Validate the requested container type is breakable BEFORE creating
        // anything, so a non-breakable request leaves no orphaned container.
        let requested_type = type_label_value(&labels);
        self.ensure_type_breakable(config, requested_type.as_deref())?;

        let (container_id, create_warnings) = self.create_issue(
            title,
            description,
            priority,
            gates,
            labels,
            content_format,
            force,
        )?;

        let (result, mut warnings) = self.bracket_container(config, &container_id, force)?;
        // Surface issue-creation warnings first, then the bracket-wiring warnings.
        let mut all_warnings = create_warnings;
        all_warnings.append(&mut warnings);
        Ok((result, all_warnings))
    }

    /// Validate that `container`'s `type:` label is one of `breakable_types`.
    fn ensure_breakable(&self, config: &PlanningConfig, container: &Issue) -> Result<()> {
        self.ensure_type_breakable(config, type_label_value(&container.labels).as_deref())
    }

    /// Validate a (possibly absent) type name against `breakable_types`.
    fn ensure_type_breakable(
        &self,
        config: &PlanningConfig,
        type_value: Option<&str>,
    ) -> Result<()> {
        match type_value {
            Some(ty) if config.breakable_types.iter().any(|b| b == ty) => Ok(()),
            Some(ty) => Err(anyhow!(
                "container type '{ty}' is not breakable; declared breakable types: {}",
                config.breakable_types.join(", ")
            )),
            None => Err(anyhow!(
                "container has no type: label; planning brackets require a breakable type (one of: {})",
                config.breakable_types.join(", ")
            )),
        }
    }

    /// Core bracket wiring shared by both entry points: create `P`, move `C`'s
    /// upstream deps onto `P`, wire `C → P`, apply the plan-review preset, and
    /// set `P`'s plan-doc location. Caller guarantees `C` is breakable.
    fn bracket_container(
        &self,
        config: &PlanningConfig,
        full_container_id: &str,
        force: bool,
    ) -> Result<(PlanResult, Vec<String>)> {
        let mut warnings = Vec::new();
        let container = self.storage.load_issue(full_container_id)?;

        // Snapshot the container's pre-existing upstream dependencies BEFORE any
        // edge mutation, so the retrofit moves exactly the original set.
        let original_deps = container.dependencies.clone();

        // 1. Create the planning node P, typed from config. It inherits the
        //    container's non-type labels (epic/milestone membership, etc.) so it
        //    is grouped with its container; the type label is replaced.
        let mut planning_labels: Vec<String> = container
            .labels
            .iter()
            .filter(|l| {
                label_utils::parse_label(l)
                    .map(|(ns, _)| ns != "type")
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        planning_labels.push(format!("type:{}", config.planning_type));

        let (planning_id, mut create_warnings) = self.create_issue(
            format!("Plan: {}", container.title),
            String::new(),
            container.priority,
            vec![],
            planning_labels,
            None,
            force,
        )?;
        warnings.append(&mut create_warnings);

        // 2. Move each pre-existing upstream dependency onto P (front of the
        //    spine), then make the container depend on P. Order mirrors
        //    `breakdown_issue`: wire the new edges first, then drop the old ones,
        //    so transitive reduction never strands an edge mid-operation.
        for dep_id in &original_deps {
            let (_, mut w) = self.add_dependency(&planning_id, dep_id)?;
            warnings.append(&mut w);
        }
        let (_, mut w) = self.add_dependency(full_container_id, &planning_id)?;
        warnings.append(&mut w);
        for dep_id in &original_deps {
            let mut w = self.remove_dependency(full_container_id, dep_id)?;
            warnings.append(&mut w);
        }

        // 3. Apply the agent plan-quality preset to P.
        let (_, mut w) = self.apply_gate_preset(
            &planning_id,
            &config.plan_gate_preset,
            None,
            false,
            false,
            &[],
        )?;
        warnings.append(&mut w);

        // 4. Set P's plan-doc location (T4 resolver / config). An inline plan IS
        //    P's own body, so nothing is attached; an external template resolves
        //    to a repo-relative path recorded as a document reference on P.
        let plan_doc_location =
            self.set_plan_doc_location(config, &planning_id, full_container_id)?;

        Ok((
            PlanResult {
                container_id: full_container_id.to_string(),
                planning_id,
                planning_type: config.planning_type.clone(),
                plan_gate_preset: config.plan_gate_preset.clone(),
                plan_doc_location,
            },
            warnings,
        ))
    }

    /// Resolve `P`'s plan-doc location from the config template and record it.
    ///
    /// Returns the resolved location string. For [`PlanDocLocation::Inline`] the
    /// plan is `P`'s own body, so no document reference is attached and the
    /// literal `"inline"` is returned. For [`PlanDocLocation::External`] the
    /// `{id}`-substituted path (resolved against the CONTAINER id, matching the
    /// `brackets:`/coverage convention that the plan belongs to `C`) is recorded
    /// as a document reference on `P` and returned.
    fn set_plan_doc_location(
        &self,
        config: &PlanningConfig,
        planning_id: &str,
        container_id: &str,
    ) -> Result<String> {
        match resolve_plan_doc_location(&config.plan_doc_location, container_id) {
            PlanDocLocation::Inline => Ok(plan_doc::INLINE_LOCATION.to_string()),
            PlanDocLocation::External(path) => {
                let path_str = path.to_string_lossy().to_string();
                let mut planning = self.storage.load_issue(planning_id)?;
                planning
                    .documents
                    .push(DocumentReference::new(path_str.clone()).with_label("plan".to_string()));
                self.storage.save_issue(planning)?;
                // Audit the state change: attaching the external plan-doc
                // reference mutates the planning node, so log an issue-updated
                // event (CLAUDE.md: all state changes must be logged). Appended
                // after the save commits so a failed write leaves no ghost event.
                self.storage
                    .append_event(&crate::domain::Event::new_issue_updated(
                        planning_id.to_string(),
                        "agent:scaffold".to_string(),
                        vec!["documents".to_string()],
                    ))?;
                Ok(path_str)
            }
        }
    }
}

/// Extract the `type:*` value from a label list, if present (pure helper).
fn type_label_value(labels: &[String]) -> Option<String> {
    labels.iter().find_map(|l| {
        label_utils::parse_label(l)
            .ok()
            .and_then(|(ns, v)| (ns == "type").then_some(v))
    })
}
