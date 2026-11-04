//! Registered responses and human rendering of the `jit archive` leaves.

use super::{
    render_archive_candidates, render_archive_execution, render_archive_plan, HumanRender,
    OutputMode,
};
use crate::domain::artifact_execution::ArchiveExecutionResult;
use crate::domain::artifact_plan::{ArchiveCandidates, ArtifactPlan};
use schemars::JsonSchema;
use serde::Serialize;

impl HumanRender for ArchiveCandidates {
    fn render_human(&self, _: OutputMode) -> String {
        render_archive_candidates(self)
    }
}

impl HumanRender for ArtifactPlan {
    fn render_human(&self, _: OutputMode) -> String {
        render_archive_plan(self)
    }
}

impl HumanRender for ArchiveExecutionResult {
    fn render_human(&self, _: OutputMode) -> String {
        render_archive_execution(self)
    }
}

#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ArchiveTargetResponse {
    Preview(ArtifactPlan),
    Executed(ArchiveExecutionResult),
}

impl HumanRender for ArchiveTargetResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        match self {
            Self::Preview(plan) => plan.render_human(mode),
            Self::Executed(result) => result.render_human(mode),
        }
    }
}
