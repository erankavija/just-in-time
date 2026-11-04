//! Human rendering for project command responses.

use super::HumanRender;
use crate::commands::ProjectRenderResult;

impl HumanRender for ProjectRenderResult {
    fn render_human(&self) -> String {
        if self.items.is_empty() {
            return "No projections declared\n".to_owned();
        }

        self.items
            .iter()
            .map(|projection| {
                format!(
                    "Rendered projection '{}' to {} ({} mode, {} style)\n",
                    projection.name, projection.target, projection.mode, projection.style
                )
            })
            .collect()
    }
}
