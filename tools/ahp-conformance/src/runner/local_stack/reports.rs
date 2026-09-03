//! Case reporting and restart orchestration for the local stack.

use super::super::model_config;
use super::super::streaming::selection;
use super::super::tools::{cancellation, replace, restart, shell, workspace};
use super::super::{drafts, suite, CaseScope, ConformanceError, Report, RunOptions};
use super::LocalStack;
use advanced_workspace_host::RunningAdvancedHost;

pub(super) async fn append_workspace_tool_cases(
    report: &mut Report,
    target: &str,
    options: &RunOptions,
    stack: &LocalStack,
) {
    if suite::case_selected(&options.selected_cases, "workspace-validation") {
        append_local_case(
            report,
            options,
            "workspace-validation",
            workspace::workspace_validation(
                target,
                options.timeout,
                &stack.workspace_root,
                &stack.selected_workspace,
                &stack.outside_workspace,
            )
            .await,
        );
    } else {
        append_local_case(report, options, "workspace-validation", Ok(String::new()));
    }

    if suite::case_selected(&options.selected_cases, "replace-string-tool") {
        append_local_case(
            report,
            options,
            "replace-string-tool",
            replace::replace_string_tool(
                target,
                options.timeout,
                &stack.selected_workspace,
                &stack.outside_workspace,
            )
            .await,
        );
    } else {
        append_local_case(report, options, "replace-string-tool", Ok(String::new()));
    }
    if suite::case_selected(&options.selected_cases, "shell-approval") {
        append_local_case(
            report,
            options,
            "shell-approval",
            shell::shell_approval(target, options.timeout, &stack.selected_workspace).await,
        );
    } else {
        append_local_case(report, options, "shell-approval", Ok(String::new()));
    }
    if suite::case_selected(&options.selected_cases, "tool-cancellation") {
        append_local_case(
            report,
            options,
            "tool-cancellation",
            cancellation::tool_cancellation(target, options.timeout, &stack.selected_workspace)
                .await,
        );
    } else {
        append_local_case(report, options, "tool-cancellation", Ok(String::new()));
    }
}

pub(super) async fn append_model_config_cases(
    report: &mut Report,
    target: &str,
    options: &RunOptions,
    stack: &LocalStack,
) {
    for name in [
        "model-catalog",
        "session-model-config",
        "session-model-default",
        "session-model-change",
        "message-model-override",
    ] {
        if !suite::case_selected(&options.selected_cases, name) {
            append_model_config_case(report, options, name, Ok(String::new()));
            continue;
        }
        let result = match name {
            "model-catalog" => model_config::model_catalog(target, options.timeout).await,
            "session-model-config" => {
                model_config::session_model_config(target, options.timeout, &stack.workspace_root)
                    .await
            }
            "session-model-default" => {
                model_config::session_model_default(target, options.timeout, stack).await
            }
            "session-model-change" => {
                model_config::session_model_change(target, options.timeout, stack).await
            }
            "message-model-override" => {
                model_config::message_model_override(target, options.timeout, stack).await
            }
            _ => Err(ConformanceError::Options(
                "unknown local model configuration case".to_owned(),
            )),
        };
        append_model_config_case(report, options, name, result);
    }
}

pub(super) async fn append_draft_cases(
    report: &mut Report,
    target: &str,
    options: &RunOptions,
    stack: &LocalStack,
) {
    for name in [
        "draft-send-lifecycle",
        "response-part-ordering",
        "draft-model-only",
        "conversation-history",
    ] {
        let result = if suite::case_selected(&options.selected_cases, name) {
            match name {
                "draft-send-lifecycle" => {
                    drafts::draft_send_lifecycle(target, options.timeout).await
                }
                "response-part-ordering" => {
                    drafts::response_part_ordering(
                        target,
                        options.timeout,
                        &stack.selected_workspace,
                    )
                    .await
                }
                "draft-model-only" => drafts::draft_model_only(target, options.timeout).await,
                "conversation-history" => {
                    drafts::conversation_history(target, options.timeout).await
                }
                _ => Err(ConformanceError::Options(
                    "unknown local chat draft case".to_owned(),
                )),
            }
        } else {
            Ok(String::new())
        };
        append_draft_case(report, options, name, result);
    }
}

pub(super) async fn append_restart_cases(
    report: &mut Report,
    options: &RunOptions,
    stack: &LocalStack,
) {
    if suite::case_selected(&options.selected_cases, "persistence-restart") {
        append_fixture_case(
            report,
            options,
            "persistence-restart",
            persist_general_restart(options, stack).await,
        );
    } else {
        append_fixture_case(report, options, "persistence-restart", Ok(String::new()));
    }
    if suite::case_selected(&options.selected_cases, "shell-approval-persistence") {
        append_local_case(
            report,
            options,
            "shell-approval-persistence",
            persist_shell_approval(options, stack).await,
        );
    } else {
        append_local_case(
            report,
            options,
            "shell-approval-persistence",
            Ok(String::new()),
        );
    }
    append_model_streaming_cases(report, options, stack).await;
    if suite::case_selected(&options.selected_cases, "session-model-restart") {
        append_model_config_case(
            report,
            options,
            "session-model-restart",
            model_config::session_model_restart(stack, options.timeout).await,
        );
    } else {
        append_model_config_case(report, options, "session-model-restart", Ok(String::new()));
    }
    if suite::case_selected(&options.selected_cases, "chat-drafts") {
        append_draft_case(
            report,
            options,
            "chat-drafts",
            drafts::chat_drafts(stack, options.timeout).await,
        );
    } else {
        append_draft_case(report, options, "chat-drafts", Ok(String::new()));
    }
}

async fn persist_general_restart(
    options: &RunOptions,
    stack: &LocalStack,
) -> Result<String, ConformanceError> {
    let first = stack.start_host().await?;
    let target = first.websocket_url();
    let retained =
        restart::create_persisted_turn(&target, options.timeout, &stack.selected_workspace).await;
    let stopped = stop(first).await;
    let retained = retained?;
    stopped?;
    let restarted = stack.start_host().await?;
    let target = restarted.websocket_url();
    let verified = restart::verify_persisted_turn(&target, options.timeout, &retained).await;
    stop(restarted).await?;
    verified?;
    Ok(
        "retains sessions, completed turns, and monotonic server sequences across restart"
            .to_owned(),
    )
}

async fn persist_shell_approval(
    options: &RunOptions,
    stack: &LocalStack,
) -> Result<String, ConformanceError> {
    let first = stack.start_host().await?;
    let target = first.websocket_url();
    let approved = shell::approve_always(&target, options.timeout, &stack.selected_workspace).await;
    stop(first).await?;
    approved?;
    let restarted = stack.start_host().await?;
    let target = restarted.websocket_url();
    let verified =
        shell::verify_persisted_approval(&target, options.timeout, &stack.selected_workspace).await;
    stop(restarted).await?;
    verified?;
    Ok(
        "persists exact-command approval and auto-confirms only that command after restart"
            .to_owned(),
    )
}

async fn stop(host: RunningAdvancedHost) -> Result<(), ConformanceError> {
    host.shutdown()
        .await
        .map_err(|error| ConformanceError::Fixture(error.to_string()))
}

fn append_local_case(
    report: &mut Report,
    options: &RunOptions,
    name: &'static str,
    result: Result<String, ConformanceError>,
) {
    if suite::case_selected(&options.selected_cases, name) {
        report.cases.push(suite::case_result(
            name,
            CaseScope::LocalFixtureWorkspaceTools,
            result,
        ));
    } else {
        report.cases.push(suite::skipped(
            name,
            CaseScope::LocalFixtureWorkspaceTools,
            "not selected",
        ));
    }
}

fn append_fixture_case(
    report: &mut Report,
    options: &RunOptions,
    name: &'static str,
    result: Result<String, ConformanceError>,
) {
    if suite::case_selected(&options.selected_cases, name) {
        report.cases.push(suite::case_result(
            name,
            CaseScope::LocalFixtureSampleHost,
            result,
        ));
    } else {
        report.cases.push(suite::skipped(
            name,
            CaseScope::LocalFixtureSampleHost,
            "not selected",
        ));
    }
}

fn append_model_config_case(
    report: &mut Report,
    options: &RunOptions,
    name: &'static str,
    result: Result<String, ConformanceError>,
) {
    if suite::case_selected(&options.selected_cases, name) {
        report.cases.push(suite::case_result(
            name,
            CaseScope::LocalFixtureModelConfiguration,
            result,
        ));
    } else {
        report.cases.push(suite::skipped(
            name,
            CaseScope::LocalFixtureModelConfiguration,
            "not selected",
        ));
    }
}

fn append_draft_case(
    report: &mut Report,
    options: &RunOptions,
    name: &'static str,
    result: Result<String, ConformanceError>,
) {
    if suite::case_selected(&options.selected_cases, name) {
        report.cases.push(suite::case_result(
            name,
            CaseScope::LocalFixtureChatDrafts,
            result,
        ));
    } else {
        report.cases.push(suite::skipped(
            name,
            CaseScope::LocalFixtureChatDrafts,
            "not selected",
        ));
    }
}

async fn append_model_streaming_cases(
    report: &mut Report,
    options: &RunOptions,
    stack: &LocalStack,
) {
    for name in [
        "responses-streaming",
        "chat-completions-streaming",
        "api-auto-selection",
        "streaming-cancellation",
    ] {
        if !suite::case_selected(&options.selected_cases, name) {
            report.cases.push(suite::skipped(
                name,
                CaseScope::LocalFixtureModelStreaming,
                "not selected",
            ));
            continue;
        }
        let result = match name {
            "responses-streaming" => selection::responses_streaming(stack, options.timeout).await,
            "chat-completions-streaming" => {
                selection::chat_completions_streaming(stack, options.timeout).await
            }
            "api-auto-selection" => selection::api_auto_selection(stack, options.timeout).await,
            "streaming-cancellation" => {
                selection::streaming_cancellation(stack, options.timeout).await
            }
            _ => Err(ConformanceError::Options(
                "unknown local model streaming case".to_owned(),
            )),
        };
        report.cases.push(suite::case_result(
            name,
            CaseScope::LocalFixtureModelStreaming,
            result,
        ));
    }
}
