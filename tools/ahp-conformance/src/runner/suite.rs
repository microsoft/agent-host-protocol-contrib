//! Case selection and aggregate suite execution.
//!
//! Internal conformance-runner module.

use super::*;

pub(super) async fn run_suite_at(
    target: String,
    options: RunOptions,
    local_fixture: bool,
) -> Result<Report, ConformanceError> {
    validate_selected_cases(&options.selected_cases, local_fixture)?;
    let mut cases = Vec::new();
    for (name, scope) in [
        ("negotiation", CaseScope::ProtocolCore),
        ("sessions", CaseScope::TextChatProfile),
        ("streaming-lifecycle", CaseScope::TextChatProfile),
        ("cancellation", CaseScope::TextChatProfile),
        ("synchronization-replay", CaseScope::TextChatProfile),
        ("errors", CaseScope::ProtocolCore),
    ] {
        if !case_selected(&options.selected_cases, name) {
            cases.push(skipped(name, scope, "not selected"));
            continue;
        }
        let result = match name {
            "negotiation" => basic::negotiation(&target, options.timeout).await,
            "sessions" => basic::sessions(&target, options.timeout).await,
            "streaming-lifecycle" => {
                turns::streaming_lifecycle(&target, options.timeout, options.model_checks).await
            }
            "cancellation" => {
                turns::cancellation(&target, options.timeout, options.model_checks).await
            }
            "synchronization-replay" => {
                advanced::synchronization_replay(&target, options.timeout, options.model_checks)
                    .await
            }
            "errors" => advanced::errors(&target, options.timeout).await,
            _ => Err(ConformanceError::Options(
                "unknown built-in case".to_owned(),
            )),
        };
        cases.push(case_result(name, scope, result));
    }
    if local_fixture && case_selected(&options.selected_cases, "upstream-error") {
        cases.push(case_result(
            "upstream-error",
            CaseScope::LocalFixtureSampleHost,
            advanced::upstream_error(&target, options.timeout).await,
        ));
    } else if local_fixture {
        cases.push(skipped(
            "upstream-error",
            CaseScope::LocalFixtureSampleHost,
            "not selected",
        ));
    }
    Ok(Report {
        target,
        model_checks: options.model_checks,
        cases,
        format: options.format,
    })
}

pub(super) fn case_selected(selected: &[String], name: &str) -> bool {
    selected.is_empty() || selected.iter().any(|case| case == name)
}

pub(super) fn validate_selected_cases(
    selected: &[String],
    local_fixture: bool,
) -> Result<(), ConformanceError> {
    const STANDARD: &[&str] = &[
        "negotiation",
        "sessions",
        "streaming-lifecycle",
        "cancellation",
        "synchronization-replay",
        "errors",
    ];
    const LOCAL_ONLY: &[&str] = &[
        "upstream-error",
        "persistence-restart",
        "workspace-validation",
        "replace-string-tool",
        "shell-approval",
        "shell-approval-persistence",
        "tool-cancellation",
        "responses-streaming",
        "chat-completions-streaming",
        "api-auto-selection",
        "streaming-cancellation",
        "model-catalog",
        "session-model-config",
        "session-model-default",
        "session-model-change",
        "message-model-override",
        "session-model-restart",
    ];

    for name in selected {
        if STANDARD.contains(&name.as_str()) {
            continue;
        }
        if LOCAL_ONLY.contains(&name.as_str()) {
            if local_fixture {
                continue;
            }
            return Err(ConformanceError::Options(format!(
                "case {name:?} requires --local-fixture"
            )));
        }
        return Err(ConformanceError::Options(format!(
            "unknown case {name:?}; use one of: {}, {}",
            STANDARD.join(", "),
            LOCAL_ONLY.join(", ")
        )));
    }
    Ok(())
}

pub(super) fn case_result(
    name: &'static str,
    scope: CaseScope,
    result: Result<String, ConformanceError>,
) -> CaseResult {
    match result {
        Ok(detail) => CaseResult {
            name,
            status: CaseStatus::Passed,
            scope,
            detail,
        },
        Err(error) => CaseResult {
            name,
            status: CaseStatus::Failed,
            scope,
            detail: error.to_string(),
        },
    }
}

pub(super) fn skipped(name: &'static str, scope: CaseScope, detail: &str) -> CaseResult {
    CaseResult {
        name,
        status: CaseStatus::Skipped,
        scope,
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_selected_cases;

    #[test]
    fn selection_rejects_unknown_and_local_only_cases_on_external_hosts() {
        assert!(validate_selected_cases(&["nope".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["upstream-error".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["persistence-restart".to_owned()], true).is_ok());
        assert!(validate_selected_cases(&["workspace-validation".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["shell-approval-persistence".to_owned()], true).is_ok());
        assert!(validate_selected_cases(&["responses-streaming".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["api-auto-selection".to_owned()], true).is_ok());
        assert!(validate_selected_cases(&["model-catalog".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["session-model-restart".to_owned()], true).is_ok());
    }
}
