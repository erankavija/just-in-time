//! Responses and human renderings of the `jit issue` commands.

use super::{
    minimal_blocked_reason, plain_issue_rows, render_issue_list, BlockedFullListResponse,
    BlockedIssue, BlockedListResponse, BlockedReason, ContainerProgressResponse, HumanRender,
    IssueChildrenResponse, IssueColumn, IssueCreateResponse, IssueListFullResponse,
    IssueListResponse, IssueSearchFullResponse, IssueSearchResponse, IssueShowListResponse,
    IssueShowResponse, IssueShowSummaryResponse, IssueStatusListResponse, IssueStatusResponse,
    IssueUpdateResponse, ListedIssue, OutputMode,
};
use crate::commands::{BatchCreateOutcome, BatchDryRunOutcome, BulkUpdateResult, ItemShowResult};
use crate::domain::identity::IssueId;
use crate::domain::queries::BlockingReason;
use crate::domain::{Issue, MinimalBlockedIssue, MinimalIssue};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write;

/// The named top-level field(s) of one issue's show response: plain text for
/// `--field`, one compact JSON object for `--fields`.
#[derive(Serialize, JsonSchema)]
#[serde(transparent)]
pub struct IssueProjection {
    value: serde_json::Value,
    #[serde(skip)]
    #[schemars(skip)]
    text: String,
}

impl IssueProjection {
    /// Project `name` from the serialized show response `shown`; `None` when
    /// the response has no such field.
    pub fn field(shown: &serde_json::Value, name: &str) -> Option<Self> {
        Some(Self {
            value: shown.get(name)?.clone(),
            text: super::project_field(shown, name)?,
        })
    }

    /// Project `names`, reporting the first one the response lacks.
    pub fn fields(
        shown: &serde_json::Value,
        names: &[String],
    ) -> Result<Self, super::UnknownFieldError> {
        Ok(Self {
            text: super::project_fields(shown, names)?,
            value: serde_json::Value::Object(
                names
                    .iter()
                    .filter_map(|name| Some((name.clone(), shown.get(name)?.clone())))
                    .collect(),
            ),
        })
    }
}

impl HumanRender for IssueProjection {
    fn render_human(&self, _: OutputMode) -> String {
        format!("{}\n", self.text)
    }
}

/// Response of `jit issue show`: what its arguments select.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum IssueShowOutput {
    /// One issue.
    Issue(Box<IssueShowResponse>),
    /// One issue under `--summary --json`.
    Summary(IssueShowSummaryResponse),
    /// Several issues, in argument order.
    Many(IssueShowListResponse),
    /// The item a `<issue>/<self-id>` argument addresses.
    Item(ItemShowResult),
    /// `--field` or `--fields`.
    Projection(IssueProjection),
}

impl HumanRender for IssueShowOutput {
    fn render_human(&self, mode: OutputMode) -> String {
        match self {
            Self::Issue(issue) => issue.render_human(mode),
            Self::Summary(summary) => format!(
                "Issue {}: {} [{}]\n",
                summary.short_id, summary.title, summary.state
            ),
            Self::Many(many) => many.items.iter().map(|r| r.render_human(mode)).collect(),
            Self::Item(item) => item.render_human(mode),
            Self::Projection(projection) => projection.render_human(mode),
        }
    }
}

impl HumanRender for IssueShowResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let mut text = format!(
            "ID: {}\nTitle: {}\nDescription: {}\nState: {}\nPriority: {}\nAssignee: {:?}\n",
            self.id, self.title, self.description, self.state, self.priority, self.assignee
        );
        if self.dependencies.is_empty() {
            text.push_str("Dependencies: None\n");
        } else {
            let met = self
                .dependencies
                .iter()
                .filter(|d| crate::domain::is_dependency_met(d.state, d.archived_from))
                .count();
            let _ = writeln!(
                text,
                "Dependencies ({met}/{} met):",
                self.dependencies.len()
            );
            for dep in &self.dependencies {
                let _ = writeln!(text, "  {} - {} [{}]", dep.short_id(), dep.title, dep.state);
            }
        }
        if self.gates.is_empty() {
            text.push_str("Gates: (none)\n");
        } else {
            text.push_str("Gates:\n");
            for gate in &self.gates {
                let last_run = gate.last_run_at.as_deref().unwrap_or("never run");
                let _ = writeln!(
                    text,
                    "  - {} [{}] (last run: {last_run})",
                    gate.key, gate.status
                );
            }
        }
        if !self.created_at.is_empty() {
            let _ = writeln!(text, "Created: {}", self.created_at);
        }
        if !self.updated_at.is_empty() {
            let _ = writeln!(text, "Updated: {}", self.updated_at);
        }
        if !self.documents.is_empty() {
            text.push_str("Documents:\n");
            for doc in &self.documents {
                let _ = write!(text, "  - {}", doc.path);
                if let Some(label) = &doc.label {
                    let _ = write!(text, " ({label})");
                }
                match &doc.commit {
                    Some(commit) => {
                        let _ = write!(text, " [{}]", &commit[..7.min(commit.len())]);
                    }
                    None => text.push_str(" [HEAD]"),
                }
                text.push('\n');
            }
        }
        text
    }
}

/// Response of `jit issue status`: a bare status for one id, the list envelope
/// for several.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum IssueStatusOutput {
    One(IssueStatusResponse),
    Many(IssueStatusListResponse),
}

impl HumanRender for IssueStatusOutput {
    fn render_human(&self, _: OutputMode) -> String {
        let lines = |statuses: &[IssueStatusResponse]| {
            statuses
                .iter()
                .map(|s| format!("{}\n", s.to_line()))
                .collect()
        };
        match self {
            Self::One(status) => lines(std::slice::from_ref(status)),
            Self::Many(many) => lines(&many.items),
        }
    }
}

impl HumanRender for IssueChildrenResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let mut text: String = self
            .items
            .iter()
            .map(|child| format!("{}\n", child.to_line()))
            .collect();
        append_dangling(&mut text, &self.extras.dangling);
        text
    }
}

impl HumanRender for ContainerProgressResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let mut text = format!(
            "{} [{}] title: {}\n",
            self.container.short_id,
            self.container.state.as_str(),
            self.container.title
        );
        self.rollup
            .to_lines()
            .iter()
            .for_each(|line| text.push_str(&format!("{line}\n")));
        append_dangling(&mut text, &self.dangling);
        text
    }
}

fn append_dangling(text: &mut String, dangling: &[crate::domain::identity::IssueId]) {
    if !dangling.is_empty() {
        let ids = dangling.iter().map(|id| id.as_str()).collect::<Vec<_>>();
        let _ = writeln!(text, "dangling: {}", ids.join(","));
    }
}

/// The issue-list text of `issues`: `header(count)` and `Total: N` around the
/// rows, or the rows alone under `quiet` (nothing when there are none).
fn list_text<I: ListedIssue>(
    header: impl FnOnce(usize) -> String,
    issues: &[I],
    columns: &[IssueColumn],
    quiet: bool,
) -> String {
    render_issue_list(
        &header(issues.len()),
        plain_issue_rows(issues),
        columns,
        quiet,
    )
}

const STATE_AND_PRIORITY: &[IssueColumn] = &[IssueColumn::State, IssueColumn::Priority];

/// Response of `jit issue list`: lean entries, or stored records under `--full`.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum IssueListing {
    Summary(IssueListResponse),
    Full(IssueListFullResponse),
}

/// Response of `jit issue search`: lean entries, or stored records under
/// `--full`, beside the echoed query.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum IssueSearchResults {
    Summary(IssueSearchResponse),
    Full(IssueSearchFullResponse),
}

impl IssueListing {
    /// The listing of `issues`: stored records under `full`, else lean entries.
    pub fn new(issues: Vec<Issue>, full: bool) -> Self {
        if full {
            Self::Full(IssueListFullResponse::new(issues))
        } else {
            Self::Summary(IssueListResponse::new(
                issues.iter().map(MinimalIssue::from).collect(),
            ))
        }
    }
}

impl HumanRender for IssueListing {
    fn render_human(&self, mode: OutputMode) -> String {
        let header = |_| "All issues (filtered):".to_owned();
        match self {
            Self::Summary(list) => list_text(header, &list.items, STATE_AND_PRIORITY, mode.quiet),
            Self::Full(list) => list_text(header, &list.items, STATE_AND_PRIORITY, mode.quiet),
        }
    }
}

/// The subset of issues a `jit query` leaf lists; it picks the human heading
/// and columns.
#[derive(Debug, Clone, Copy)]
pub enum QuerySubset {
    Available,
    Strategic,
    Closed,
}

impl QuerySubset {
    fn header(self) -> &'static str {
        match self {
            Self::Available => "Available issues (unassigned, unblocked):",
            Self::Strategic => "Strategic issues:",
            Self::Closed => "Closed issues (Done or Rejected):",
        }
    }

    fn columns(self) -> &'static [IssueColumn] {
        match self {
            Self::Available | Self::Strategic => &[IssueColumn::Priority],
            Self::Closed => &[IssueColumn::State],
        }
    }
}

/// Response of `jit query available|strategic|closed`: an [`IssueListing`]
/// under the subset's own human heading.
#[derive(Serialize, JsonSchema)]
#[serde(transparent)]
pub struct SubsetListing {
    listing: IssueListing,
    #[serde(skip)]
    #[schemars(skip)]
    subset: QuerySubset,
}

impl SubsetListing {
    pub fn new(subset: QuerySubset, issues: Vec<Issue>, full: bool) -> Self {
        Self {
            listing: IssueListing::new(issues, full),
            subset,
        }
    }
}

impl HumanRender for SubsetListing {
    fn render_human(&self, mode: OutputMode) -> String {
        let (header, columns) = (|_| self.subset.header().to_owned(), self.subset.columns());
        match &self.listing {
            IssueListing::Summary(list) => list_text(header, &list.items, columns, mode.quiet),
            IssueListing::Full(list) => list_text(header, &list.items, columns, mode.quiet),
        }
    }
}

/// Response of `jit query blocked`: lean entries, or reason-structured entries
/// under `--full`. Human rows list each reason in its display form, which
/// differs from the lean JSON reason strings.
#[derive(Serialize, JsonSchema)]
#[serde(transparent)]
pub struct BlockedListing {
    payload: BlockedPayload,
    #[serde(skip)]
    #[schemars(skip)]
    rows: Vec<(MinimalIssue, Vec<String>)>,
}

#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
enum BlockedPayload {
    Summary(BlockedListResponse),
    Full(BlockedFullListResponse),
}

impl BlockedListing {
    /// Map the command-layer blocked issues and their reasons into both shapes'
    /// reason forms.
    pub fn new(blocked: &[(Issue, Vec<BlockingReason>)], full: bool) -> Self {
        let payload = if full {
            BlockedPayload::Full(BlockedFullListResponse::new(
                blocked
                    .iter()
                    .map(|(issue, reasons)| BlockedIssue {
                        issue: MinimalIssue::from(issue),
                        blocked_reasons: reasons.iter().map(BlockedReason::from).collect(),
                    })
                    .collect(),
            ))
        } else {
            BlockedPayload::Summary(BlockedListResponse::new(
                blocked
                    .iter()
                    .map(|(issue, reasons)| {
                        MinimalBlockedIssue::from((
                            issue,
                            reasons.iter().map(minimal_blocked_reason).collect(),
                        ))
                    })
                    .collect(),
            ))
        };
        let rows = blocked
            .iter()
            .map(|(issue, reasons)| {
                (
                    MinimalIssue::from(issue),
                    reasons.iter().map(ToString::to_string).collect(),
                )
            })
            .collect();
        Self { payload, rows }
    }
}

impl HumanRender for BlockedListing {
    fn render_human(&self, mode: OutputMode) -> String {
        render_issue_list(
            "Blocked issues:",
            self.rows
                .iter()
                .map(|(issue, reasons)| (issue, reasons.clone())),
            &[IssueColumn::Priority],
            mode.quiet,
        )
    }
}

impl HumanRender for IssueSearchResults {
    fn render_human(&self, mode: OutputMode) -> String {
        let header = |count| format!("Found {count} issue(s):");
        match self {
            Self::Summary(list) => list_text(header, &list.items, STATE_AND_PRIORITY, mode.quiet),
            Self::Full(list) => list_text(header, &list.items, STATE_AND_PRIORITY, mode.quiet),
        }
    }
}

/// `line` and a newline, or nothing under `-q`.
fn confirmation(line: impl std::fmt::Display, mode: OutputMode) -> String {
    if mode.quiet {
        String::new()
    } else {
        format!("{line}\n")
    }
}

impl HumanRender for IssueCreateResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        if mode.quiet {
            return format!("{}\n", self.issue.id);
        }
        format!("Created issue: {}\n", self.issue.id)
            + &super::render_validation_findings(&self.findings, None).stdout
    }
}

/// Response of `jit issue update`: one issue's confirmation, or the
/// `--filter` batch result.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum IssueUpdateOutput {
    One(IssueUpdateResponse),
    Bulk(BulkUpdateResult),
}

impl HumanRender for IssueUpdateOutput {
    fn render_human(&self, mode: OutputMode) -> String {
        let result = match self {
            Self::One(updated) => {
                return confirmation(format!("Updated issue: {}", updated.id), mode)
            }
            Self::Bulk(result) => result,
        };
        let summary = &result.summary;
        let mut text = if summary.total_modified > 0 {
            confirmation(
                format!("Modified {} issue(s)", summary.total_modified),
                mode,
            )
        } else {
            String::new()
        };
        let mut section = |title: String, entries: &[(IssueId, String)]| {
            if !entries.is_empty() {
                let _ = writeln!(text, "\n{title}");
                for (id, detail) in entries {
                    let _ = writeln!(text, "  • {}: {detail}", id.short_id());
                }
            }
        };
        section(
            format!("Skipped {} issue(s):", summary.total_skipped),
            &result.skipped,
        );
        section(
            format!("Failed {} issue(s):", summary.total_errors),
            &result.errors,
        );
        if summary.total_matched > 0 {
            let _ = writeln!(
                text,
                "\nSummary: {}/{} succeeded ({:.0}%)",
                summary.total_modified,
                summary.total_matched,
                (summary.total_modified as f64 / summary.total_matched as f64) * 100.0
            );
        } else {
            text.push_str("No issues matched filter\n");
        }
        text
    }
}

/// Response of `jit issue delete`.
#[derive(Serialize, JsonSchema)]
pub struct IssueDeleteResponse {
    pub id: IssueId,
    /// Always true: a refused deletion is a failure.
    pub deleted: bool,
}

impl HumanRender for IssueDeleteResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        confirmation(format!("Deleted issue: {}", self.id), mode)
    }
}

/// Response of an issue lifecycle mutation (`assign`, `claim`, `unassign`,
/// `reject`, `release`, `claim-next`): the stored record it returns.
#[derive(Serialize, JsonSchema)]
#[serde(transparent)]
pub struct IssueLifecycleResponse {
    issue: Issue,
    #[serde(skip)]
    #[schemars(skip)]
    line: String,
}

impl IssueLifecycleResponse {
    /// `issue`, confirmed in human mode by `line`.
    pub fn new(issue: Issue, line: impl Into<String>) -> Self {
        Self {
            issue,
            line: line.into(),
        }
    }
}

impl HumanRender for IssueLifecycleResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        confirmation(&self.line, mode)
    }
}

/// Response of `jit issue batch-create`: the validation result under
/// `--dry-run`, else the created ids.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum IssueBatchCreateOutput {
    DryRun(BatchDryRunOutcome),
    Created(BatchCreateResponse),
}

/// The issues a batch created.
#[derive(Serialize, JsonSchema)]
pub struct BatchCreateResponse {
    /// Each entry's symbolic key mapped to its created issue's full id.
    pub ids: BTreeMap<String, IssueId>,
    #[serde(skip)]
    #[schemars(skip)]
    in_input_order: Vec<(String, IssueId)>,
}

impl From<BatchCreateOutcome> for BatchCreateResponse {
    fn from(outcome: BatchCreateOutcome) -> Self {
        Self {
            ids: outcome.key_to_id.iter().cloned().collect(),
            in_input_order: outcome.key_to_id,
        }
    }
}

impl HumanRender for IssueBatchCreateOutput {
    fn render_human(&self, mode: OutputMode) -> String {
        match self {
            Self::DryRun(outcome) => format!(
                "Valid batch: {} issue(s), {} dependency edge(s); no changes made.\n",
                outcome.issue_count, outcome.dependency_count
            ),
            Self::Created(created) => created.in_input_order.iter().fold(
                confirmation(format!("Created {} issue(s):", created.ids.len()), mode),
                |text, (key, id)| text + &format!("  {key} -> {id}\n"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::Outcome;
    use crate::output::{emit_outcome, OutputMode};

    fn emitted(listing: IssueListing, quiet: bool) -> String {
        let mut out = Vec::new();
        let outcome = Outcome::Success {
            response: listing,
            message: None,
            warnings: Vec::new(),
        };
        emit_outcome(
            &outcome,
            OutputMode { json: false, quiet },
            &mut out,
            &mut std::io::sink(),
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn test_emit_outcome_empty_issue_listing_prints_header_and_total_unless_quiet() {
        let empty = || IssueListing::Summary(IssueListResponse::new(Vec::new()));

        assert_eq!(
            emitted(empty(), false),
            "All issues (filtered):\n\nTotal: 0\n"
        );
        assert_eq!(emitted(empty(), true), "");
    }

    fn issues(titles: &[&str]) -> Vec<Issue> {
        titles
            .iter()
            .map(|title| crate::test_utils::fixture_issue((*title).to_owned(), String::new()))
            .collect()
    }

    #[test]
    fn test_subset_listing_renders_each_subset_under_its_own_heading_and_columns() {
        let listed = issues(&["One"]);
        let render = |subset| {
            SubsetListing::new(subset, listed.clone(), false).render_human(OutputMode::default())
        };

        let (available, closed) = (render(QuerySubset::Available), render(QuerySubset::Closed));

        assert!(available.starts_with(QuerySubset::Available.header()));
        assert!(closed.starts_with(QuerySubset::Closed.header()));
        assert!(available.contains(&listed[0].priority.to_string()));
        assert!(closed.contains(&listed[0].state.to_string()));
        assert!(!available.contains(&listed[0].state.to_string()));
    }

    #[test]
    fn test_subset_listing_serializes_as_the_plain_listing() {
        let listed = issues(&["One"]);

        let subset = SubsetListing::new(QuerySubset::Strategic, listed.clone(), true);

        assert_eq!(
            serde_json::to_value(subset).unwrap(),
            serde_json::to_value(IssueListing::new(listed, true)).unwrap()
        );
    }

    #[test]
    fn test_blocked_listing_human_rows_show_display_reasons_while_json_keeps_lean_strings() {
        let blocker = crate::test_utils::issue_id(1);
        let reasons = vec![
            BlockingReason::Dependency {
                id: blocker,
                title: "Dep".to_owned(),
                state: crate::domain::State::Ready,
            },
            BlockingReason::Gate {
                key: "tests".parse().unwrap(),
                status: crate::domain::GateStatus::Pending,
            },
        ];
        let blocked = vec![(issues(&["Waiting"]).remove(0), reasons.clone())];

        let listing = BlockedListing::new(&blocked, false);

        let human = listing.render_human(OutputMode::default());
        reasons
            .iter()
            .for_each(|reason| assert!(human.contains(&format!("- {reason}")), "{human}"));
        let json = serde_json::to_value(&listing).unwrap();
        assert_eq!(
            json["issues"][0]["blocked_reasons"],
            serde_json::json!(reasons
                .iter()
                .map(minimal_blocked_reason)
                .collect::<Vec<_>>())
        );
    }

    #[test]
    fn test_blocked_listing_full_serializes_structured_reasons_for_each_reason_kind() {
        let reasons = vec![BlockingReason::Gate {
            key: "tests".parse().unwrap(),
            status: crate::domain::GateStatus::Pending,
        }];
        let blocked = vec![(issues(&["Waiting"]).remove(0), reasons)];

        let json = serde_json::to_value(BlockedListing::new(&blocked, true)).unwrap();

        assert_eq!(json["issues"][0]["blocked_reasons"][0]["type"], "gate");
    }

    #[test]
    fn test_issue_projection_serializes_as_the_projected_value() {
        let shown = serde_json::json!({"state": "ready", "title": "T"});

        let field = IssueProjection::field(&shown, "state").unwrap();
        let fields = IssueProjection::fields(&shown, &["title".to_owned()]).unwrap();

        assert_eq!(serde_json::to_value(&field).unwrap(), "ready");
        assert_eq!(
            serde_json::to_value(&fields).unwrap(),
            serde_json::json!({"title": "T"})
        );
        assert_eq!(field.render_human(OutputMode::default()), "ready\n");
    }
}
