//! Human rendering of the `jit item` responses.

use super::{HumanRender, OutputMode};
use crate::commands::{ItemListResult, ItemShowResult};

impl HumanRender for ItemListResult {
    fn render_human(&self, mode: OutputMode) -> String {
        if self.items.is_empty() {
            return if mode.quiet {
                String::new()
            } else {
                "No addressable items found\n".to_owned()
            };
        }
        self.items
            .iter()
            .map(|item| format!("{}  [{}]  {}\n", item.qualified_id, item.kind, item.text))
            .collect()
    }
}

impl HumanRender for ItemShowResult {
    fn render_human(&self, _: OutputMode) -> String {
        let item = &self.item;
        let owner = match (&self.issue_full_id, &self.issue_title) {
            (Some(full_id), Some(title)) => format!("Issue:        {full_id} | {title}"),
            // A project-scoped item (`@/<kind>/<self-id>`) has no owning issue.
            _ => "Scope:        @ (project)".to_owned(),
        };
        [
            format!("Qualified id: {}", item.qualified_id),
            format!("Kind:         {}", item.kind),
            format!("Self id:      {}", item.self_id),
            owner,
            format!("Text:         {}", item.text),
        ]
        .iter()
        .map(|line| format!("{line}\n"))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_human_empty_list_quiet_writes_nothing() {
        let list = ItemListResult::new(Vec::new());

        let rendered = list.render_human(OutputMode {
            json: false,
            quiet: true,
        });

        assert!(rendered.is_empty());
    }

    #[test]
    fn test_render_human_empty_list_not_quiet_names_no_items() {
        let list = ItemListResult::new(Vec::new());

        let rendered = list.render_human(OutputMode {
            json: false,
            quiet: false,
        });

        assert_eq!(rendered, "No addressable items found\n");
    }
}
