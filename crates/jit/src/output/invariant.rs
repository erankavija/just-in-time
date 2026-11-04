//! Human rendering of the `jit invariant` responses.

use super::{HumanRender, OutputMode};
use crate::commands::InvariantCheckResult;

impl HumanRender for InvariantCheckResult {
    fn render_human(&self, _: OutputMode) -> String {
        if self.items.is_empty() {
            return "No enforcement drift\n".to_owned();
        }
        self.items
            .iter()
            .map(|finding| format!("error: {}\n", finding.message()))
            .collect()
    }
}
