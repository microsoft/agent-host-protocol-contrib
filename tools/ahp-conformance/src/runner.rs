use std::time::Duration;

use serde::Serialize;
use thiserror::Error;
use url::Url;

/// Output representation requested by the command-line runner.
#[derive(Clone, Copy, Debug)]
pub enum OutputFormat {
    /// A compact report for people.
    Text,
    /// One JSON document for CI systems.
    Json,
}

/// Settings for an observable conformance run.
#[derive(Clone)]
pub struct RunOptions {
    /// WebSocket endpoint. Omit only with [`run_local_suite`].
    pub target: Option<String>,
    /// Maximum time for each request or expected event.
    pub timeout: Duration,
    /// Names of cases to run. An empty list selects every applicable case.
    pub selected_cases: Vec<String>,
    /// Enables optional exact-output assertions using the documented fixture syntax.
    pub model_checks: bool,
    /// OAuth bearer token pushed to protected resources advertised by the selected agent.
    pub auth_token: Option<String>,
    /// Requested report format.
    pub format: OutputFormat,
}

/// Failures that prevent a conformance runner from starting.
#[derive(Debug, Error)]
pub enum ConformanceError {
    /// A required runner setting was absent or invalid.
    #[error("invalid conformance options: {0}")]
    Options(String),
    /// An HTTP/WebSocket transport step failed.
    #[error("WebSocket transport failure: {0}")]
    Transport(#[from] tokio_tungstenite::tungstenite::Error),
    /// A JSON frame was invalid.
    #[error("invalid JSON frame: {0}")]
    Json(#[from] serde_json::Error),
    /// An operation timed out.
    #[error("timed out while {0}")]
    Timeout(String),
    /// The peer closed the connection.
    #[error("the AHP peer closed the WebSocket connection")]
    Closed,
    /// A local fixture could not start or stop.
    #[error("local fixture failure: {0}")]
    Fixture(String),
    /// A local file operation failed.
    #[error("local data operation failed: {0}")]
    Io(#[from] std::io::Error),
}

pub(crate) fn redact_target(target: &str) -> String {
    let Ok(mut url) = Url::parse(target) else {
        return "<invalid WebSocket target>".to_owned();
    };
    if !url.username().is_empty() {
        let _ = url.set_username("redacted");
    }
    if url.password().is_some() {
        let _ = url.set_password(Some("redacted"));
    }
    if url.query().is_some() {
        url.set_query(Some("redacted"));
    }
    url.to_string()
}

/// A complete machine-readable result.
#[derive(Debug, Serialize)]
pub struct Report {
    target: String,
    model_checks: bool,
    cases: Vec<CaseResult>,
    #[serde(skip)]
    format: OutputFormat,
}

impl Report {
    /// True if any executed case failed.
    pub fn failed(&self) -> bool {
        self.cases
            .iter()
            .any(|case| matches!(case.status, CaseStatus::Failed))
    }

    /// Returns the number of cases in the report, including skipped or
    /// not-applicable cases.
    #[must_use]
    pub fn case_count(&self) -> usize {
        self.cases.len()
    }

    /// Renders the requested human- or machine-readable report.
    pub fn render(&self) -> String {
        match self.format {
            OutputFormat::Json => match serde_json::to_string_pretty(self) {
                Ok(output) => format!("{output}\n"),
                Err(error) => format!(
                    "{{\"error\":\"failed to serialize conformance report: {}\"}}\n",
                    error
                ),
            },
            OutputFormat::Text => {
                let mut output = format!(
                    "AHP conformance target: {}\noptional model checks: {}\n",
                    self.target, self.model_checks
                );
                for case in &self.cases {
                    output.push_str(&format!(
                        "{:<7} {} [{}] — {}\n",
                        case.status.as_str(),
                        case.name,
                        case.scope.as_str(),
                        case.detail
                    ));
                }
                output
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct CaseResult {
    name: &'static str,
    status: CaseStatus,
    scope: CaseScope,
    detail: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum CaseScope {
    ProtocolCore,
    TextChatProfile,
    LocalFixtureSampleHost,
    LocalFixtureWorkspaceTools,
    LocalFixtureModelStreaming,
    LocalFixtureModelConfiguration,
    LocalFixtureChatDrafts,
}

impl CaseScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::ProtocolCore => "protocol-core",
            Self::TextChatProfile => "text-chat-profile",
            Self::LocalFixtureSampleHost => "local-fixture-sample-host",
            Self::LocalFixtureWorkspaceTools => "local-fixture-workspace-tools",
            Self::LocalFixtureModelStreaming => "local-fixture-model-streaming",
            Self::LocalFixtureModelConfiguration => "local-fixture-model-configuration",
            Self::LocalFixtureChatDrafts => "local-fixture-chat-drafts",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum CaseStatus {
    Passed,
    Failed,
    Skipped,
    NotApplicable,
}

impl CaseStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "PASS",
            Self::Failed => "FAIL",
            Self::Skipped => "SKIP",
            Self::NotApplicable => "N/A",
        }
    }
}

pub(super) struct ScenarioCheck {
    name: &'static str,
    status: CaseStatus,
    detail: String,
}

impl ScenarioCheck {
    pub(super) fn passed(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: CaseStatus::Passed,
            detail: detail.into(),
        }
    }

    pub(super) fn failed(name: &'static str, error: impl std::fmt::Display) -> Self {
        Self {
            name,
            status: CaseStatus::Failed,
            detail: error.to_string(),
        }
    }

    pub(super) fn not_applicable(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: CaseStatus::NotApplicable,
            detail: detail.into(),
        }
    }
}

pub(super) struct ScenarioRun {
    checks: Vec<ScenarioCheck>,
    abort_detail: Option<String>,
}

impl ScenarioRun {
    pub(super) fn new() -> Self {
        Self {
            checks: Vec::new(),
            abort_detail: None,
        }
    }

    pub(super) fn push(&mut self, check: ScenarioCheck) {
        self.checks.push(check);
    }

    pub(super) fn abort(mut self, error: impl std::fmt::Display) -> Self {
        self.abort_detail = Some(error.to_string());
        self
    }
}

/// Runs cases suitable for an already-running host.
///
/// Protocol cases avoid exact model wording. Set `model_checks` only when the
/// target is intentionally following the documented deterministic syntax.
pub async fn run_suite(options: RunOptions) -> Result<Report, ConformanceError> {
    let target = options
        .target
        .clone()
        .ok_or_else(|| ConformanceError::Options("--target is required".to_owned()))?;
    suite::run_suite_at(target, options, false).await
}

/// Starts the local deterministic model and host, then runs every applicable
/// case including the persistence/restart probe.
///
/// The transient redb database is put below `data_dir`, which should normally
/// be a `target/` directory in the current workspace. It is removed on success
/// or failure.
mod advanced;
mod authentication;
mod basic;
mod drafts;
mod local_stack;
mod model_config;
mod probe;
mod session_contracts;
mod streaming;
mod suite;
mod tools;
mod turns;

pub use local_stack::run_local_suite;

use probe::Probe;

#[cfg(test)]
mod tests {
    use super::{redact_target, CaseResult, CaseScope, CaseStatus, OutputFormat, Report};

    #[test]
    fn report_targets_redact_query_credentials() {
        assert_eq!(
            redact_target("ws://localhost:60460/?tkn=secret"),
            "ws://localhost:60460/?redacted"
        );
    }

    fn report(format: OutputFormat) -> Report {
        Report {
            target: "ws://example.test/ahp".to_owned(),
            model_checks: false,
            cases: vec![CaseResult {
                name: "negotiation",
                status: CaseStatus::Passed,
                scope: CaseScope::ProtocolCore,
                detail: "works".to_owned(),
            }],
            format,
        }
    }

    #[test]
    fn reports_include_machine_and_human_scope_labels() {
        assert!(report(OutputFormat::Text)
            .render()
            .contains("negotiation [protocol-core]"));
        assert!(report(OutputFormat::Json)
            .render()
            .contains("\"scope\": \"protocol_core\""));
    }

    #[test]
    fn reports_optional_capabilities_as_not_applicable() {
        let report = Report {
            target: "ws://example.test/ahp".to_owned(),
            model_checks: false,
            cases: vec![CaseResult {
                name: "additional-chat",
                status: CaseStatus::NotApplicable,
                scope: CaseScope::TextChatProfile,
                detail: "multipleChats was not advertised".to_owned(),
            }],
            format: OutputFormat::Json,
        };
        assert!(report.render().contains("\"status\": \"not_applicable\""));
        assert!(!report.failed());
    }

    #[test]
    fn reports_label_local_workspace_tool_cases() {
        let report = Report {
            target: "ws://example.test/ahp".to_owned(),
            model_checks: true,
            cases: vec![CaseResult {
                name: "workspace-validation",
                status: CaseStatus::Passed,
                scope: CaseScope::LocalFixtureWorkspaceTools,
                detail: "works".to_owned(),
            }],
            format: OutputFormat::Json,
        };
        assert!(report
            .render()
            .contains("\"scope\": \"local_fixture_workspace_tools\""));
    }

    #[test]
    fn reports_label_local_model_streaming_cases() {
        let report = Report {
            target: "ws://example.test/ahp".to_owned(),
            model_checks: true,
            cases: vec![CaseResult {
                name: "responses-streaming",
                status: CaseStatus::Passed,
                scope: CaseScope::LocalFixtureModelStreaming,
                detail: "works".to_owned(),
            }],
            format: OutputFormat::Text,
        };
        assert!(report
            .render()
            .contains("responses-streaming [local-fixture-model-streaming]"));
    }

    #[test]
    fn reports_label_local_chat_draft_cases() {
        let report = Report {
            target: "ws://example.test/ahp".to_owned(),
            model_checks: true,
            cases: vec![CaseResult {
                name: "chat-drafts",
                status: CaseStatus::Passed,
                scope: CaseScope::LocalFixtureChatDrafts,
                detail: "works".to_owned(),
            }],
            format: OutputFormat::Text,
        };
        assert!(report
            .render()
            .contains("chat-drafts [local-fixture-chat-drafts]"));
    }
}
