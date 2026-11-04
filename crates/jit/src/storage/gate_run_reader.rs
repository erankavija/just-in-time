//! The one gate-run record reader both storage backends share.
//!
//! A backend supplies only [`GateRunTree`]; the clutter, kind and parse rules
//! live here once, so a change to the record layout changes one reader.

use crate::domain::identity::{GateRunId, IssueId};
use crate::domain::GateRunResult;
use crate::repository_state::VirtualPath;
use crate::storage::GateRunNotFoundError;
use anyhow::{Context, Result};

/// What a backend finds at one path, read without following a symlink.
pub(crate) enum TreeNode<T> {
    /// Nothing is there, or an ancestor is absent.
    Absent,
    /// An occupant of another kind (a symlink, a special file, a file where a
    /// directory belongs, or the reverse), or an ancestor that is not an
    /// ordinary directory.
    OtherKind,
    Present(T),
}

/// The backend seam the reader runs over.
pub(crate) trait GateRunTree {
    /// The UTF-8 child names of the ordinary directory at `directory`.
    fn list_directory(&self, directory: &VirtualPath) -> Result<TreeNode<Vec<String>>>;

    /// The bytes of the ordinary file at `file`.
    fn read_regular_file(&self, file: &VirtualPath) -> Result<TreeNode<Vec<u8>>>;
}

/// Every recorded run of `issue_id`.
///
/// A child of the gate-run root that is not a run directory named by a run id,
/// or a run directory holding no record, is clutter and lists nothing. The root
/// itself must be an ordinary directory when present.
pub(crate) fn list_gate_runs_for_issue(
    tree: &impl GateRunTree,
    issue_id: &IssueId,
) -> Result<Vec<GateRunResult>> {
    let names = match tree.list_directory(&VirtualPath::GATE_RUNS)? {
        TreeNode::Absent => return Ok(Vec::new()),
        TreeNode::OtherKind => anyhow::bail!(
            "Gate run root at {} must be an ordinary directory",
            VirtualPath::GATE_RUNS.repository_relative()
        ),
        TreeNode::Present(names) => names,
    };
    names
        .iter()
        .filter_map(|name| GateRunId::try_from(name.as_str()).ok())
        .filter_map(
            |run_id| match tree.list_directory(&VirtualPath::gate_run_directory(&run_id)) {
                Ok(TreeNode::Present(_)) => read_record(tree, &run_id).transpose(),
                Ok(TreeNode::Absent | TreeNode::OtherKind) => None,
                Err(error) => Some(Err(error)),
            },
        )
        .filter(|run| !matches!(run, Ok(run) if &run.issue_id != issue_id))
        .collect()
}

/// The run `run_id` of `issue_id`.
///
/// # Errors
///
/// [`GateRunNotFoundError`] when no record exists or it belongs to another
/// issue; an error naming the path when the root, the run directory or the
/// record is of the wrong kind, or the record does not parse.
pub(crate) fn load_gate_run_result(
    tree: &impl GateRunTree,
    issue_id: &IssueId,
    run_id: &GateRunId,
) -> Result<GateRunResult> {
    let not_found = || GateRunNotFoundError::new(run_id.clone()).into();
    for directory in [
        VirtualPath::GATE_RUNS,
        VirtualPath::gate_run_directory(run_id),
    ] {
        match tree.list_directory(&directory)? {
            TreeNode::Absent => return Err(not_found()),
            TreeNode::OtherKind => anyhow::bail!(
                "Gate run directory at {} must be an ordinary directory",
                directory.repository_relative()
            ),
            TreeNode::Present(_) => {}
        }
    }
    read_record(tree, run_id)?
        .filter(|run| &run.issue_id == issue_id)
        .ok_or_else(not_found)
}

fn read_record(tree: &impl GateRunTree, run_id: &GateRunId) -> Result<Option<GateRunResult>> {
    let path = VirtualPath::gate_run_record(run_id);
    match tree.read_regular_file(&path)? {
        TreeNode::Absent => Ok(None),
        TreeNode::OtherKind => anyhow::bail!(
            "Gate run result at {} must be an ordinary file",
            path.repository_relative()
        ),
        TreeNode::Present(bytes) => serde_json::from_slice(&bytes).map(Some).with_context(|| {
            format!(
                "Failed to deserialize gate run result at {}",
                path.repository_relative()
            )
        }),
    }
}
