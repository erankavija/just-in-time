//! Human rendering of the `jit events` responses.

use super::{EventListResponse, HumanRender, OutputMode};

impl HumanRender for EventListResponse {
    /// One serialized event per line. An event's derived `Serialize` cannot fail.
    fn render_human(&self, _: OutputMode) -> String {
        self.items
            .iter()
            .map(|event| format!("{}\n", serde_json::to_string(event).unwrap_or_default()))
            .collect()
    }
}
