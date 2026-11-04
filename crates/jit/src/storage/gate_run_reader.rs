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

/// Every recorded run of `issue_id`, from one listing of its directory.
///
/// A child not named `<run-id>.json`, and a record of another issue, is clutter
/// and lists nothing. The directory must be an ordinary directory when present.
pub(crate) fn list_gate_runs_for_issue(
    tree: &impl GateRunTree,
    issue_id: &IssueId,
) -> Result<Vec<GateRunResult>> {
    let Some(names) = list_issue_directory(tree, issue_id)? else {
        return Ok(Vec::new());
    };
    names
        .iter()
        .filter_map(|name| name.strip_suffix(".json"))
        .filter_map(|stem| GateRunId::try_from(stem).ok())
        .filter_map(|run_id| read_record(tree, issue_id, &run_id).transpose())
        .filter(|run| !matches!(run, Ok(run) if &run.issue_id != issue_id))
        .collect()
}

/// The run `run_id` of `issue_id`.
///
/// # Errors
///
/// [`GateRunNotFoundError`] when no record exists or it belongs to another
/// issue; an error naming the path when the issue's directory or the record is
/// of the wrong kind, or the record does not parse.
pub(crate) fn load_gate_run_result(
    tree: &impl GateRunTree,
    issue_id: &IssueId,
    run_id: &GateRunId,
) -> Result<GateRunResult> {
    let not_found = || GateRunNotFoundError::new(run_id.clone()).into();
    list_issue_directory(tree, issue_id)?.ok_or_else(not_found)?;
    read_record(tree, issue_id, run_id)?
        .filter(|run| &run.issue_id == issue_id)
        .ok_or_else(not_found)
}

/// The child names of `issue_id`'s gate-run directory; `None` when it is absent.
fn list_issue_directory(
    tree: &impl GateRunTree,
    issue_id: &IssueId,
) -> Result<Option<Vec<String>>> {
    let directory = VirtualPath::gate_run_issue_directory(issue_id);
    match tree.list_directory(&directory)? {
        TreeNode::Absent => Ok(None),
        TreeNode::OtherKind => anyhow::bail!(
            "Gate run directory at {} must be an ordinary directory",
            directory.repository_relative()
        ),
        TreeNode::Present(names) => Ok(Some(names)),
    }
}

fn read_record(
    tree: &impl GateRunTree,
    issue_id: &IssueId,
    run_id: &GateRunId,
) -> Result<Option<GateRunResult>> {
    let path = VirtualPath::gate_run_record(issue_id, run_id);
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
