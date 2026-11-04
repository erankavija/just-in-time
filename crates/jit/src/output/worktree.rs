use super::{HumanRender, OutputMode, WorktreeListResponse, WorktreeStoreDivergenceResponse};
use crate::commands::worktree::WorktreeInfo;

impl HumanRender for WorktreeInfo {
    fn render_human(&self, _: OutputMode) -> String {
        let kind = if self.is_main_worktree {
            "main worktree"
        } else {
            "secondary worktree"
        };
        format!(
            "Worktree Information:\n  ID:         {}\n  Branch:     {}\n  Root:       {}\n  Type:       {kind}\n  Common dir: {}\n",
            self.worktree_id, self.branch, self.root_path, self.common_dir
        )
    }
}

impl HumanRender for WorktreeListResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let header = format!(
            "{:<16} {:<25} {:<50} {:>6}\n{}\n",
            "WORKTREE ID",
            "BRANCH",
            "PATH",
            "CLAIMS",
            "-".repeat(100)
        );
        self.items.iter().fold(header, |text, entry| {
            format!(
                "{text}{:<16} {:<25} {:<50} {:>6}\n",
                entry.worktree_id, entry.branch, entry.path, entry.active_claims
            )
        })
    }
}

impl HumanRender for WorktreeStoreDivergenceResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let header = format!(
            "Checkout store:  {}\nReference store: {}\n\n",
            self.extras.checkout_store,
            self.extras
                .reference_store
                .as_deref()
                .unwrap_or("none (this checkout has no other store to compare against)")
        );
        if self.items.is_empty() {
            format!("{header}No divergent records.\n")
        } else {
            let table = self.items.iter().fold(
                format!(
                    "{header}{:<8} {:<16} ID\n{}\n",
                    "RECORD",
                    "CLASS",
                    "-".repeat(72)
                ),
                |text, entry| {
                    format!(
                        "{text}{:<8} {:<16} {}\n",
                        entry.record.as_str(),
                        entry.class.as_str(),
                        entry.id
                    )
                },
            );
            format!("{table}\n{} divergence(s)\n", self.items.len())
        }
    }
}
