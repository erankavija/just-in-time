//! Human rendering of the `jit doc` responses.

use super::{ArtifactConformanceResponse, ArtifactDirectoryResponse, HumanRender, OutputMode};
use crate::commands::{
    AssetListResult, DocumentAddResult, DocumentContentResult, DocumentDiffResult, DocumentHistory,
    DocumentListResult, DocumentRemoveResult, LinkCheckResult,
};
use crate::document::{Asset, AssetType};
use std::fmt::Write;

impl HumanRender for DocumentAddResult {
    fn render_human(&self, _: OutputMode) -> String {
        let document = &self.document;
        let mut rendered = format!(
            "{} document reference on issue {}\n  Path: {}\n",
            if self.updated { "Updated" } else { "Added" },
            self.issue_id,
            document.path
        );
        for (name, value) in [
            ("Commit", document.commit.as_deref()),
            ("Label", document.label.as_deref()),
            ("Type", document.doc_type.as_deref()),
            ("Format", document.format.as_deref()),
        ] {
            if let Some(value) = value {
                let _ = writeln!(rendered, "  {name}: {value}");
            }
        }
        if !document.assets.is_empty() {
            let _ = writeln!(rendered, "  Assets: {} discovered", document.assets.len());
        }
        rendered
    }
}

impl HumanRender for DocumentListResult {
    fn render_human(&self, _: OutputMode) -> String {
        let issue = &self.extras.issue_id;
        if self.items.is_empty() {
            return format!("No document references for issue {issue}\n");
        }
        let mut rendered = format!("Document references for issue {issue}:\n");
        for doc in &self.items {
            let _ = write!(rendered, "  - {}", doc.path);
            if let Some(label) = &doc.label {
                let _ = write!(rendered, " ({label})");
            }
            match &doc.commit {
                Some(commit) => {
                    let _ = write!(rendered, " [{}]", &commit[..7.min(commit.len())]);
                }
                None => rendered.push_str(" [HEAD]"),
            }
            if let Some(doc_type) = &doc.doc_type {
                let _ = write!(rendered, " <{doc_type}>");
            }
            rendered.push('\n');
        }
        let _ = writeln!(rendered, "\nTotal: {}", self.count());
        rendered
    }
}

impl HumanRender for DocumentRemoveResult {
    fn render_human(&self, _: OutputMode) -> String {
        format!(
            "Removed document reference {} from issue {}\n",
            self.path, self.issue_id
        )
    }
}

impl HumanRender for DocumentContentResult {
    fn render_human(&self, _: OutputMode) -> String {
        let mut rendered = format!("Document: {}\n", self.path);
        if let Some(label) = &self.label {
            let _ = writeln!(rendered, "Label: {label}");
        }
        let _ = writeln!(rendered, "Commit: {}", self.commit);
        if let Some(doc_type) = &self.doc_type {
            let _ = writeln!(rendered, "Type: {doc_type}");
        }
        let _ = write!(rendered, "\n---\n\n{}\n", self.content);
        rendered
    }
}

/// The bare path, so the command composes into a shell substitution.
impl HumanRender for ArtifactDirectoryResponse {
    fn render_human(&self, _: OutputMode) -> String {
        format!("{}\n", self.directory)
    }
}

impl HumanRender for ArtifactConformanceResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let areas = &self.extras.areas;
        let mut rendered = format!(
            "Scanned areas: {}\n",
            if areas.is_empty() {
                "none declared".to_owned()
            } else {
                areas.join(", ")
            }
        );
        if self.items.is_empty() {
            rendered.push_str("No artifact sits outside its owning issue's canonical directory.\n");
        } else {
            let _ = writeln!(rendered, "Artifacts ({}):", self.items.len());
        }
        for artifact in &self.items {
            let _ = writeln!(rendered, "  {}", artifact.path);
            let _ = match (&artifact.canonical_directory, &artifact.issue_id) {
                (Some(directory), Some(issue_id)) => writeln!(
                    rendered,
                    "    {} -> {directory} (issue {issue_id})",
                    artifact.status
                ),
                // A name without a short id reached the report through the
                // references naming it, so the owner is what stayed unsettled.
                _ if artifact.short_id.is_empty() => writeln!(
                    rendered,
                    "    {} (several issues reference it)",
                    artifact.status
                ),
                _ => writeln!(
                    rendered,
                    "    {} (no issue answers to {})",
                    artifact.status, artifact.short_id
                ),
            };
        }
        rendered
    }
}

impl HumanRender for DocumentHistory {
    fn render_human(&self, _: OutputMode) -> String {
        let mut rendered = format!("History for {}:\n\n", self.path);
        for commit in &self.commits {
            let _ = write!(
                rendered,
                "commit {}\nAuthor: {}\nDate:   {}\n\n    {}\n\n",
                commit.sha, commit.author, commit.date, commit.message
            );
        }
        rendered
    }
}

impl HumanRender for DocumentDiffResult {
    fn render_human(&self, _: OutputMode) -> String {
        self.diff.clone()
    }
}

impl HumanRender for AssetListResult {
    fn render_human(&self, _: OutputMode) -> String {
        let scope = &self.extras;
        let mut rendered = format!(
            "Assets for document {} (issue {}):\n",
            scope.document_path,
            scope.issue_id.short_id()
        );
        if self.items.is_empty() {
            rendered.push_str("  No assets found for this document\n");
            return rendered;
        }
        let located = |asset: &Asset, with_mime: bool| {
            let exists = asset
                .resolved_path
                .as_ref()
                .is_some_and(|path| scope.worktree_root.join(path).exists());
            let mut line = format!(
                "  {} {}\n",
                if exists { "ok" } else { "missing" },
                asset.original_path
            );
            if let Some(resolved) = &asset.resolved_path {
                let _ = writeln!(line, "     → {}", resolved.display());
            }
            if let (true, Some(mime)) = (with_mime, &asset.mime_type) {
                let _ = writeln!(line, "     MIME: {mime}");
            }
            line
        };
        let of = |asset_type: AssetType, shared: bool| {
            self.items
                .iter()
                .filter(move |a| a.asset_type == asset_type && a.is_shared == shared)
        };
        let sections = [
            (
                "Per-document assets",
                of(AssetType::Local, false)
                    .map(|a| located(a, true))
                    .collect::<String>(),
            ),
            (
                "Shared assets",
                of(AssetType::Local, true)
                    .map(|a| located(a, false))
                    .collect(),
            ),
            (
                "External URLs",
                self.items
                    .iter()
                    .filter(|a| a.asset_type == AssetType::External)
                    .map(|a| format!("  remote {}\n", a.original_path))
                    .collect(),
            ),
            (
                "Missing assets",
                self.items
                    .iter()
                    .filter(|a| a.asset_type == AssetType::Missing)
                    .map(|a| {
                        let expected = a.resolved_path.as_ref().map_or(String::new(), |path| {
                            format!("     Expected at: {}\n", path.display())
                        });
                        format!("  missing {}\n{expected}", a.original_path)
                    })
                    .collect(),
            ),
        ];
        for (heading, lines) in sections.iter().filter(|(_, lines)| !lines.is_empty()) {
            let _ = write!(rendered, "\n{heading}:\n{lines}");
        }
        let summary = &scope.summary;
        let _ = writeln!(
            rendered,
            "\nSummary: {} total ({} per-doc, {} shared, {} external, {} missing)",
            summary.total, summary.per_doc, summary.shared, summary.external, summary.missing
        );
        rendered
    }
}

impl HumanRender for LinkCheckResult {
    fn render_human(&self, _: OutputMode) -> String {
        let summary = &self.summary;
        let mut rendered = format!(
            "Checking {} document(s) in scope '{}'...\n\n",
            summary.total_documents, self.scope
        );
        for (heading, findings) in [
            ("Errors found", &self.errors),
            ("Warnings", &self.link_warnings),
        ] {
            if findings.is_empty() {
                continue;
            }
            let _ = writeln!(rendered, "{heading} ({}):", findings.len());
            for finding in findings {
                let _ = writeln!(
                    rendered,
                    "  {} ({}): {}",
                    finding["document"].as_str().unwrap_or(""),
                    finding["type"].as_str().unwrap_or(""),
                    finding["message"].as_str().unwrap_or("")
                );
            }
            rendered.push('\n');
        }
        if self.errors.is_empty() && self.link_warnings.is_empty() {
            rendered.push_str("All documents valid\n");
        }
        let _ = writeln!(
            rendered,
            "Summary: {} document(s) checked, {} error(s), {} warning(s)",
            summary.total_documents, summary.errors, summary.warnings
        );
        rendered
    }
}
