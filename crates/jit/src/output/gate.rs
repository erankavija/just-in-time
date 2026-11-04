//! Typed gate definition and preset responses and human rendering.

use super::{
    render_gate_run_status, GateCheckAllResponse, GateDefinition, GateFindingsResponse,
    GateFlatReportResponse, GateListResponse, GatePresetListResponse, GateRunHistoryResponse,
    GateRunLine, GateRunSummary, HumanRender, OutputMode,
};
use crate::declarations::GateChecker;
use crate::domain::identity::{GateKey, IssueId};
use crate::domain::{GateStatus, GateVerdictOrigin};
use crate::gate_presets::GatePresetDefinition;
use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Serialize, Serializer};
use std::collections::HashMap;
use std::fmt::Write;

#[derive(Serialize, JsonSchema)]
pub struct GateMutationResponse {
    pub key: GateKey,
    pub message: String,
}

impl GateMutationResponse {
    pub fn new(key: GateKey, action: &str) -> Self {
        Self {
            message: format!("{action} gate '{key}'"),
            key,
        }
    }
}

impl HumanRender for GateMutationResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        if mode.quiet {
            String::new()
        } else {
            format!("{}\n", self.message)
        }
    }
}

/// The complete registry declaration returned by show and update.
#[derive(Serialize, JsonSchema)]
pub struct GateDefinitionResponse {
    version: u32,
    #[serde(flatten)]
    definition: GateDefinition,
    priority: u32,
    reserved: HashMap<String, serde_json::Value>,
    #[serde(skip)]
    updated: bool,
}

impl GateDefinitionResponse {
    pub fn shown(gate: crate::declarations::GateDefinition) -> Self {
        Self {
            version: gate.version,
            priority: gate.priority,
            reserved: gate.reserved.clone(),
            definition: gate.into(),
            updated: false,
        }
    }

    pub fn updated(gate: crate::declarations::GateDefinition) -> Self {
        Self {
            updated: true,
            ..Self::shown(gate)
        }
    }
}

impl HumanRender for GateDefinitionResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let gate = &self.definition;
        if self.updated {
            return if mode.quiet {
                String::new()
            } else {
                format!("Updated gate '{}'\n", gate.key)
            };
        }
        let mut text = format!(
            "Gate: {}\n  Title: {}\n  Description: {}\n  Stage: {}\n  Mode: {}\n",
            gate.key, gate.title, gate.description, gate.stage, gate.mode,
        );
        if let Some(checker) = &gate.checker {
            text.push_str(&render_checker(checker, false));
        }
        text
    }
}

impl HumanRender for GateListResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let heading = if mode.quiet {
            ""
        } else if self.items.is_empty() {
            "No gates defined\n"
        } else {
            "Gates:\n"
        };
        let rows: String = self
            .items
            .iter()
            .map(|gate| {
                format!(
                    "  {} - {} ({}, {})\n",
                    gate.key, gate.title, gate.stage, gate.mode
                )
            })
            .collect();
        format!("{heading}{rows}")
    }
}

#[derive(Serialize, JsonSchema)]
pub struct GateAddResponse {
    issue_id: String,
    added: Vec<GateKey>,
    already_exist: Vec<GateKey>,
    message: String,
}

impl GateAddResponse {
    pub fn new(issue_id: String, added: Vec<GateKey>, already_exist: Vec<GateKey>) -> Self {
        Self {
            message: format!("Added {} gate(s) to issue {}", added.len(), issue_id),
            issue_id,
            added,
            already_exist,
        }
    }
}

impl HumanRender for GateAddResponse {
    fn render_human(&self, mode: OutputMode) -> String {
        let mut text = String::new();
        if !self.added.is_empty() {
            if !mode.quiet {
                let _ = writeln!(text, "{}:", self.message);
            }
            for key in &self.added {
                let _ = writeln!(text, "  • {key}");
            }
        }
        if !self.already_exist.is_empty() {
            let _ = writeln!(
                text,
                "Already required ({} gate(s)):",
                self.already_exist.len()
            );
            for key in &self.already_exist {
                let _ = writeln!(text, "  • {key}");
            }
        }
        text
    }
}

impl HumanRender for GatePresetListResponse {
    fn render_human(&self, _: OutputMode) -> String {
        if self.items.is_empty() {
            return "No gate presets available\n".to_owned();
        }
        self.items
            .iter()
            .map(|preset| {
                let word = if preset.gate_count == 1 {
                    "gate"
                } else {
                    "gates"
                };
                format!(
                    "{} - {} ({} {})\n",
                    preset.name, preset.description, preset.gate_count, word
                )
            })
            .collect()
    }
}

#[derive(Serialize, JsonSchema)]
pub struct GatePresetShowResponse {
    name: String,
    description: String,
    gates: Vec<GateDefinition>,
}

impl From<GatePresetDefinition> for GatePresetShowResponse {
    fn from(preset: GatePresetDefinition) -> Self {
        Self {
            name: preset.name,
            description: preset.description,
            gates: preset
                .gates
                .into_iter()
                .map(|gate| GateDefinition::from(gate.to_gate()))
                .collect(),
        }
    }
}

impl HumanRender for GatePresetShowResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let mut text = format!(
            "Preset: {}\nDescription: {}\n\nGates:\n",
            self.name, self.description
        );
        for gate in &self.gates {
            let _ = writeln!(
                text,
                "  {} - {} ({}:{})",
                gate.key, gate.title, gate.stage, gate.mode
            );
            if let Some(checker) = &gate.checker {
                text.push_str(&render_checker(checker, true));
            }
        }
        text
    }
}

/// One checker line: the preset listing indents it under the gate and drops
/// the `Checker:` label; the plain listing labels it.
fn render_checker(checker: &GateChecker, preset: bool) -> String {
    let (preset_line, plain_line) = match checker {
        GateChecker::Exec {
            command,
            timeout_seconds,
            working_dir,
            ..
        } => {
            let body = format!("    Command: {command}\n    Timeout: {timeout_seconds}s\n");
            let dir = working_dir
                .as_ref()
                .map(|dir| format!("    Working dir: {dir}\n"))
                .unwrap_or_default();
            (body.clone(), format!("  Checker:\n{body}{dir}"))
        }
        GateChecker::RepositoryValidation => (
            "    Built-in: repository validation\n".to_owned(),
            "  Checker: repository validation (built-in)\n".to_owned(),
        ),
        GateChecker::IssueValidation => (
            "    Built-in: issue validation\n".to_owned(),
            "  Checker: issue validation (built-in)\n".to_owned(),
        ),
        GateChecker::LabelTargetValidation { label_namespace } => (
            format!("    Built-in: label-target validation ({label_namespace}:)\n"),
            format!(
                "  Checker: label-target validation (built-in, namespace: {label_namespace})\n"
            ),
        ),
        GateChecker::RuleValidation { rule } => (
            format!("    Built-in: rule validation ({rule})\n"),
            format!("  Checker: rule validation (built-in, rule: {rule})\n"),
        ),
        GateChecker::ReviewPlaceholder => (
            "    Built-in: WARNING — external review placeholder\n".to_owned(),
            "  Checker: WARNING — external review placeholder\n".to_owned(),
        ),
    };
    if preset {
        preset_line
    } else {
        plain_line
    }
}

#[derive(Serialize, JsonSchema)]
pub struct GatePresetCreateResponse {
    pub name: String,
    pub path: String,
}

impl HumanRender for GatePresetCreateResponse {
    fn render_human(&self, _: OutputMode) -> String {
        format!("Created preset '{}' at {}\n", self.name, self.path)
    }
}

#[derive(Serialize, JsonSchema)]
pub struct GatePresetSuccess {
    pub issue_id: String,
    pub gates_added: Vec<GateKey>,
    pub already_existed: Vec<GateKey>,
}

#[derive(Serialize, JsonSchema)]
pub struct GatePresetError {
    pub issue_id: String,
    pub error: String,
}

#[derive(Serialize, JsonSchema)]
pub struct GatePresetApplyResponse {
    pub preset: String,
    pub success: Vec<GatePresetSuccess>,
    pub errors: Vec<GatePresetError>,
}

impl HumanRender for GatePresetApplyResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let mut text = String::new();
        if !self.success.is_empty() {
            let _ = writeln!(
                text,
                "Applied preset '{}' to {} issue(s):",
                self.preset,
                self.success.len()
            );
            for result in &self.success {
                let keys = result
                    .gates_added
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(text, "  {} - gates added: {}", result.issue_id, keys);
            }
        }
        if !self.errors.is_empty() {
            let _ = writeln!(text, "\nErrors ({}):", self.errors.len());
            for error in &self.errors {
                let _ = writeln!(text, "  {} - {}", error.issue_id, error.error);
            }
        }
        text
    }
}

/// A response beside the issue its human rendering names; it serializes and
/// publishes its schema as `T`.
pub struct ForIssue<T> {
    pub issue_id: IssueId,
    pub response: T,
}

impl<T: Serialize> Serialize for ForIssue<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.response.serialize(serializer)
    }
}

impl<T: JsonSchema> JsonSchema for ForIssue<T> {
    fn schema_name() -> String {
        T::schema_name()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        T::json_schema(generator)
    }
}

/// Payload of `jit gate status` when the selected gate has no recorded run.
#[derive(Serialize, JsonSchema)]
pub struct GateNeverRun {
    pub key: GateKey,
    pub issue_id: IssueId,
}

impl GateNeverRun {
    /// The one-line explanation, as the `--json` message and the human text.
    pub fn text(&self) -> String {
        render_gate_run_status(GateRunLine::NeverRun, &self.key, &self.issue_id)
    }
}

impl HumanRender for GateNeverRun {
    fn render_human(&self, _: OutputMode) -> String {
        format!("{}\n", self.text())
    }
}

/// Response of `jit gate status`: the shape follows the selected view.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum GateStatusOutput {
    Latest(Box<GateRunSummary>),
    History(ForIssue<GateRunHistoryResponse>),
    Report(GateFlatReportResponse),
    Findings(GateFindingsResponse),
    NeverRun(GateNeverRun),
}

impl HumanRender for GateStatusOutput {
    fn render_human(&self, mode: OutputMode) -> String {
        match self {
            Self::Latest(run) => render_run_details(run),
            Self::History(history) => history.render_human(mode),
            Self::Report(report) => report.render_human(mode),
            Self::Findings(findings) => findings.render_human(mode),
            Self::NeverRun(never_run) => never_run.render_human(mode),
        }
    }
}

fn render_run_details(run: &GateRunSummary) -> String {
    let mut text = format!(
        "Gate '{}' last run: {} (exit code: {})\n",
        run.key,
        run.status,
        run.exit_code
            .map_or_else(|| "n/a".to_owned(), |code| code.to_string())
    );
    let mut line = |label: &str, value: Option<String>| {
        if let Some(value) = value {
            let _ = writeln!(text, "  {label}: {value}");
        }
    };
    line("Duration", run.duration_ms.map(|ms| format!("{ms}ms")));
    line(
        "Command",
        Some(run.command.clone()).filter(|command| !command.is_empty()),
    );
    line("Branch", run.branch.clone());
    line("Commit", run.commit.clone());
    match &run.origin {
        GateVerdictOrigin::Reused {
            source_issue,
            source_run,
        } => {
            let _ = writeln!(
                text,
                "  Verdict: taken from issue {source_issue}, run {source_run} over the same declared inputs"
            );
        }
        GateVerdictOrigin::Attested => {
            let _ = writeln!(
                text,
                "  Attested by {} at {}",
                run.by
                    .as_ref()
                    .map_or_else(|| "an unnamed attestor".to_owned(), ToString::to_string),
                run.started_at
            );
        }
        _ => {}
    }
    if let Some(findings) = &run.findings {
        let _ = writeln!(
            text,
            "  Findings: {} (verdict: {})",
            findings.findings.len(),
            or_placeholder(&findings.verdict, "unknown")
        );
    }
    for (stream, output) in [("stdout", &run.stdout), ("stderr", &run.stderr)] {
        if let Some(output) = output.as_deref().filter(|output| !output.is_empty()) {
            let _ = writeln!(
                text,
                "  {stream}:\n    {}",
                output.lines().collect::<Vec<_>>().join("\n    ")
            );
        }
    }
    text
}

fn or_placeholder<'a>(value: &'a str, placeholder: &'a str) -> &'a str {
    if value.is_empty() {
        placeholder
    } else {
        value
    }
}

impl HumanRender for ForIssue<GateRunHistoryResponse> {
    fn render_human(&self, mode: OutputMode) -> String {
        let (issue, runs) = (&self.issue_id, &self.response.items);
        let heading = match (mode.quiet, runs.is_empty()) {
            (true, _) => String::new(),
            (false, true) => format!("No matching gate runs for issue {issue}\n"),
            (false, false) => format!("Gate run history for issue {issue}:\n"),
        };
        runs.iter()
            .fold(heading, |text, run| text + &render_run_details(run))
    }
}

impl HumanRender for ForIssue<GateCheckAllResponse> {
    fn render_human(&self, mode: OutputMode) -> String {
        let (issue, gates, summary) = (&self.issue_id, &self.response.items, &self.response.extras);
        if gates.is_empty() {
            return if mode.quiet {
                String::new()
            } else {
                format!("No required gates to inspect for issue {issue}\n")
            };
        }
        let heading = if mode.quiet {
            String::new()
        } else {
            format!("Gate readiness for issue {issue}:\n")
        };
        let runs = summary.results.iter().map(render_run_details);
        // Gates without a recorded run, so a strict nonzero exit is explained.
        let unrun = gates
            .iter()
            .filter(|gate| summary.results.iter().all(|run| run.key != gate.key))
            .map(|gate| match gate.status {
                GateStatus::Pending => {
                    GateNeverRun {
                        key: gate.key.clone(),
                        issue_id: issue.clone(),
                    }
                    .text()
                        + "\n"
                }
                GateStatus::Passed => format!("Gate '{}' status: passed\n", gate.key),
                GateStatus::Failed => format!("Gate '{}' status: failed\n", gate.key),
            });
        heading + &runs.chain(unrun).collect::<String>()
    }
}

impl HumanRender for GateFlatReportResponse {
    fn render_human(&self, _: OutputMode) -> String {
        [&self.stdout, &self.stderr]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect()
    }
}

impl HumanRender for GateFindingsResponse {
    fn render_human(&self, _: OutputMode) -> String {
        if !self.has_findings {
            return format!(
                "{} verdict: n/a findings: 0 (no machine-readable findings block)\n",
                self.key
            );
        }
        let mut text = format!(
            "{} verdict: {} summary: {} findings: {}\n",
            self.key,
            or_placeholder(self.verdict.as_deref().unwrap_or_default(), "unknown"),
            or_placeholder(self.summary.as_deref().unwrap_or_default(), "-"),
            self.findings.len(),
        );
        for finding in &self.findings {
            let classifications = [finding.disposition.as_deref(), finding.origin.as_deref()]
                .into_iter()
                .flatten()
                .map(|value| format!(" [{value}]"))
                .collect::<String>();
            let locator = match (&finding.file, finding.line) {
                (Some(file), Some(line)) => format!(" ({file}:{line})"),
                (Some(file), None) => format!(" ({file})"),
                _ => String::new(),
            };
            // JSON encoding keeps arbitrary reference strings unambiguous.
            let references = if finding.references.is_empty() {
                String::new()
            } else {
                let encoded = serde_json::to_string(&finding.references)
                    .expect("serializing a string array cannot fail");
                format!(" references: {encoded}")
            };
            let _ = writeln!(
                text,
                "{} [{}]{classifications} {}{locator}{references}",
                or_placeholder(&finding.id, "-"),
                or_placeholder(&finding.severity, "-"),
                finding.summary
            );
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::{GateMode, GateStage};
    use crate::gate_presets::GateTemplate;
    use serde_json::json;

    #[test]
    fn test_gate_definition_response_preserves_complete_declaration_payload() {
        let gate = crate::declarations::GateDefinition {
            version: 1,
            key: "sample".parse().unwrap(),
            title: "Sample".to_owned(),
            description: "A declaration".to_owned(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(GateChecker::RepositoryValidation),
            inputs: None,
            priority: 12,
            reserved: HashMap::from([("extension".to_owned(), json!("preserved"))]),
        };
        let expected = serde_json::to_value(&gate).unwrap();
        assert_eq!(
            serde_json::to_value(GateDefinitionResponse::shown(gate.clone())).unwrap(),
            expected
        );
        assert_eq!(
            serde_json::to_value(GateDefinitionResponse::updated(gate)).unwrap(),
            expected
        );
    }

    #[test]
    fn test_gate_preset_show_response_preserves_preset_payload() {
        let preset = GatePresetDefinition {
            name: "sample".to_owned(),
            description: "A preset".to_owned(),
            gates: vec![GateTemplate {
                key: "review".parse().unwrap(),
                title: "Review".to_owned(),
                description: "Human review".to_owned(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
            }],
        };
        let expected = serde_json::to_value(&preset).unwrap();
        assert_eq!(
            serde_json::to_value(GatePresetShowResponse::from(preset)).unwrap(),
            expected
        );
    }

    mod findings_text {
        use super::super::*;
        use crate::declarations::GateStage;
        use crate::domain::gate_findings::{GateFinding, GateFindings};
        use crate::domain::{GateRunResult, GateRunStatus, GATE_RUN_SCHEMA_VERSION};

        fn render(references: Vec<String>) -> String {
            let run = GateRunResult {
                schema_version: GATE_RUN_SCHEMA_VERSION,
                run_id: crate::test_utils::gate_run_id(1),
                gate_key: "review".parse().unwrap(),
                stage: GateStage::Postcheck,
                issue_id: crate::test_utils::issue_id(7),
                commit: None,
                branch: None,
                status: GateRunStatus::Failed,
                started_at: chrono::Utc::now(),
                completed_at: None,
                duration_ms: None,
                exit_code: Some(1),
                stdout: String::new(),
                stderr: String::new(),
                command: "review".to_string(),
                by: None,
                message: None,
                findings: Some(GateFindings {
                    verdict: "fail".to_string(),
                    summary: "one defect".to_string(),
                    findings: vec![GateFinding {
                        id: "F1".to_string(),
                        severity: "high".to_string(),
                        disposition: Some("blocking".to_string()),
                        origin: Some("issue-impact".to_string()),
                        summary: "unsafe write".to_string(),
                        file: Some("src/storage.rs".to_string()),
                        line: Some(12),
                        references,
                    }],
                }),
                inputs_digest: None,
                origin: crate::domain::GateVerdictOrigin::Executed,
                evaluation_stability: crate::domain::GateEvaluationStability::Stable,
            };
            GateFindingsResponse::of_run(&run).render_human(OutputMode::default())
        }

        #[test]
        fn test_render_gate_findings_text_exposes_references_exactly() {
            let rendered = render(vec![
                "@/inv/atomic-writes".to_string(),
                "checker:opaque value".to_string(),
            ]);

            assert!(
                rendered.contains(r#"references: ["@/inv/atomic-writes","checker:opaque value"]"#)
            );
        }

        #[test]
        fn test_render_gate_findings_text_json_encoding_distinguishes_collisions() {
            let comma_value = render(vec!["a, b".to_string()]);
            let two_values = render(vec!["a".to_string(), "b".to_string()]);
            let empty_value = render(vec![String::new()]);

            assert!(comma_value.contains(r#"references: ["a, b"]"#));
            assert!(two_values.contains(r#"references: ["a","b"]"#));
            assert!(empty_value.contains(r#"references: [""]"#));
            assert_ne!(comma_value, two_values);
            assert_ne!(empty_value, render(Vec::new()));
        }

        #[test]
        fn test_render_gate_findings_text_omits_empty_references() {
            assert!(!render(Vec::new()).contains("references:"));
        }
    }

    #[test]
    fn test_gate_never_run_payload_names_gate_and_issue_not_a_bare_null() {
        let never_run = GateNeverRun {
            key: "tests".parse().unwrap(),
            issue_id: crate::test_utils::issue_id(7),
        };

        let payload = serde_json::to_value(GateStatusOutput::NeverRun(never_run)).unwrap();

        assert_eq!(
            payload,
            json!({
                "key": "tests",
                "issue_id": crate::test_utils::issue_id(7).to_string(),
            })
        );
    }

    #[test]
    fn test_render_run_details_attestation_line_carries_rfc3339_started_at() {
        use crate::domain::{GateEvaluationStability, GateRunResult, GateRunStatus};
        use chrono::TimeZone;
        let started_at = chrono::Utc
            .with_ymd_and_hms(2026, 11, 4, 12, 30, 5)
            .unwrap();
        let run = GateRunResult {
            schema_version: crate::domain::GATE_RUN_SCHEMA_VERSION,
            run_id: crate::test_utils::gate_run_id(1),
            gate_key: "review".parse().unwrap(),
            stage: crate::declarations::GateStage::Postcheck,
            issue_id: crate::test_utils::issue_id(7),
            commit: None,
            branch: None,
            status: GateRunStatus::Passed,
            started_at,
            completed_at: None,
            duration_ms: None,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            command: String::new(),
            by: Some("human:alice".parse().unwrap()),
            message: None,
            findings: None,
            inputs_digest: None,
            origin: GateVerdictOrigin::Attested,
            evaluation_stability: GateEvaluationStability::Stable,
        };

        let text = render_run_details(&GateRunSummary::full(&run));

        assert!(text.contains(&format!(
            "  Attested by human:alice at {}\n",
            started_at.to_rfc3339()
        )));
        assert!(text.contains("at 2026-11-04T12:30:05+00:00\n"));
    }
}
