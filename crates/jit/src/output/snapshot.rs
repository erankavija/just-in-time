//! The `jit snapshot export` success response.

use super::{HumanRender, OutputMode};
use crate::commands::SnapshotExportResult;
use schemars::JsonSchema;
use serde::Serialize;

/// The exported snapshot's path, contents, format, and optional archive size.
#[derive(Serialize, JsonSchema)]
pub struct SnapshotExportResponse(pub SnapshotExportResult);

impl HumanRender for SnapshotExportResponse {
    fn render_human(&self, _mode: OutputMode) -> String {
        let mut rendered = format!(
            "Snapshot exported to: {}\n  {} issues\n  {} documents\n",
            self.0.path, self.0.issue_count, self.0.document_count
        );
        if let Some(size) = self.0.size_bytes {
            rendered.push_str(&format!("  Archive: {size} bytes\n"));
        }
        rendered
    }
}
