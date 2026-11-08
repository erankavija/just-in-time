//! Registered responses and human rendering of the `jit archive` leaves.

use super::{
    render_archive_advisory, render_archive_candidates, render_archive_execution,
    render_archive_plan, HumanRender, OutputMode,
};
use crate::domain::archive_advisory::{ArchiveExecutionReport, ArchivePreview};
use crate::domain::artifact_plan::ArchiveCandidates;
use schemars::JsonSchema;
use serde::Serialize;

impl HumanRender for ArchiveCandidates {
    fn render_human(&self, _: OutputMode) -> String {
        render_archive_candidates(self)
    }
}

impl HumanRender for ArchivePreview {
    fn render_human(&self, _: OutputMode) -> String {
        render_archive_plan(&self.plan) + &render_archive_advisory(&self.advisory)
    }
}

impl HumanRender for ArchiveExecutionReport {
    fn render_human(&self, _: OutputMode) -> String {
        render_archive_execution(&self.result) + &render_archive_advisory(&self.advisory)
    }
}

#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ArchiveTargetResponse {
    Preview(ArchivePreview),
    Executed(ArchiveExecutionReport),
}

impl HumanRender for ArchiveTargetResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        match self {
            Self::Preview(plan) => plan.render_human(mode),
            Self::Executed(result) => result.render_human(mode),
        }
    }
}
