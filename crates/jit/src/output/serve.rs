//! The `jit serve` response and its human rendering.

use super::{HumanRender, OutputMode};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Serialize;
use std::path::PathBuf;

/// Where a started server serves the web UI from.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebUiSource {
    Filesystem,
    Embedded,
}

/// What `jit serve` did, tagged by `status`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ServeResponse {
    Stopped {
        pid: u32,
        port: u16,
    },
    NotRunning,
    /// `log_file` and `started_at` accompany a `--status` report.
    Running {
        pid: u32,
        port: u16,
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        log_file: Option<PathBuf>,
        #[serde(skip_serializing_if = "Option::is_none")]
        started_at: Option<DateTime<Utc>>,
    },
    Started {
        pid: u32,
        port: u16,
        url: String,
        log_file: PathBuf,
        web_ui: bool,
        web_ui_source: WebUiSource,
        #[serde(skip)]
        web_dir: Option<String>,
    },
    Exited {
        port: u16,
        exit_code: Option<i32>,
    },
}

impl HumanRender for ServeResponse {
    fn render_human(&self, _mode: OutputMode) -> String {
        match self {
            Self::Stopped { pid, port } => {
                format!("Server stopped (was PID {pid} on port {port})\n")
            }
            Self::NotRunning => "Server is not running.\n".to_owned(),
            Self::Running {
                pid,
                url,
                started_at: Some(started_at),
                ..
            } => format!("Server is running: {url} (PID {pid}, started {started_at})\n"),
            Self::Running { pid, url, .. } => {
                format!("Server is already running on {url} (PID {pid})\n")
            }
            Self::Started {
                pid,
                url,
                log_file,
                web_dir,
                ..
            } => {
                let web = web_dir.as_ref().map_or_else(
                    || "(embedded assets)".to_owned(),
                    |dir| format!("(from {dir})"),
                );
                format!(
                    "Server started on {url} (PID {pid})\n  API: {url}/api\n  Web: {url}/ {web}\n  Log: {}\n",
                    log_file.display()
                )
            }
            Self::Exited { .. } => String::new(),
        }
    }
}
