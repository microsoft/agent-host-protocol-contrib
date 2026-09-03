//! Case selection and aggregate suite execution.
//!
//! Internal conformance-runner module.

use std::collections::HashMap;

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scenario {
    Transport,
    Negotiation,
    Authentication,
    Sessions,
    SessionContracts,
    Streaming,
    Cancellation,
    Synchronization,
    Errors,
}

#[derive(Clone, Copy)]
struct CaseSpec {
    name: &'static str,
    group: &'static str,
    scope: CaseScope,
    scenario: Scenario,
}

const EXTERNAL_CASES: &[CaseSpec] = &[
    CaseSpec {
        name: "ping-before-initialize",
        group: "transport",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::Transport,
    },
    CaseSpec {
        name: "ping-after-initialize",
        group: "transport",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::Transport,
    },
    CaseSpec {
        name: "initialize-version",
        group: "negotiation",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::Negotiation,
    },
    CaseSpec {
        name: "initialize-root-snapshot",
        group: "negotiation",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::Negotiation,
    },
    CaseSpec {
        name: "initialize-unsupported-version",
        group: "negotiation",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::Negotiation,
    },
    CaseSpec {
        name: "authentication-invalid-resource",
        group: "authentication",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::Authentication,
    },
    CaseSpec {
        name: "authentication-advertised-resource",
        group: "authentication",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::Authentication,
    },
    CaseSpec {
        name: "session-default-chat",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Sessions,
    },
    CaseSpec {
        name: "unknown-metadata-ignored",
        group: "sessions",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "initial-subscription-snapshots",
        group: "sessions",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "root-session-added",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "session-duplicate-error",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "session-summary-shape",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "chat-subscription-snapshot",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Sessions,
    },
    CaseSpec {
        name: "session-listing",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Sessions,
    },
    CaseSpec {
        name: "session-pagination-limit",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "session-pagination-cursor",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "session-pagination-invalid-cursor",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "session-list-order",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "fetch-turns-invalid-cursor",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "additional-chat",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Sessions,
    },
    CaseSpec {
        name: "root-session-removed",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "disposed-session-absent",
        group: "sessions",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "unsubscribe-stops-delivery",
        group: "sessions",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::SessionContracts,
    },
    CaseSpec {
        name: "turn-started",
        group: "streaming-lifecycle",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Streaming,
    },
    CaseSpec {
        name: "response-part-markdown",
        group: "streaming-lifecycle",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Streaming,
    },
    CaseSpec {
        name: "response-deltas",
        group: "streaming-lifecycle",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Streaming,
    },
    CaseSpec {
        name: "response-part-before-delta",
        group: "streaming-lifecycle",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Streaming,
    },
    CaseSpec {
        name: "turn-complete",
        group: "streaming-lifecycle",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Streaming,
    },
    CaseSpec {
        name: "root-session-summary-changed",
        group: "streaming-lifecycle",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Streaming,
    },
    CaseSpec {
        name: "session-summary-identity-omitted",
        group: "streaming-lifecycle",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Streaming,
    },
    CaseSpec {
        name: "cancellation-terminal",
        group: "cancellation",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Cancellation,
    },
    CaseSpec {
        name: "cancellation-quiescence",
        group: "cancellation",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Cancellation,
    },
    CaseSpec {
        name: "cancellation-snapshot",
        group: "cancellation",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Cancellation,
    },
    CaseSpec {
        name: "multi-client-live-delivery",
        group: "synchronization-replay",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Synchronization,
    },
    CaseSpec {
        name: "reconnect-action-replay",
        group: "synchronization-replay",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Synchronization,
    },
    CaseSpec {
        name: "replay-turn-lifecycle",
        group: "synchronization-replay",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Synchronization,
    },
    CaseSpec {
        name: "subscribe-retains-all-turns",
        group: "synchronization-replay",
        scope: CaseScope::TextChatProfile,
        scenario: Scenario::Synchronization,
    },
    CaseSpec {
        name: "jsonrpc-parse-error",
        group: "errors",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::Errors,
    },
    CaseSpec {
        name: "jsonrpc-method-not-found",
        group: "errors",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::Errors,
    },
    CaseSpec {
        name: "jsonrpc-invalid-params",
        group: "errors",
        scope: CaseScope::ProtocolCore,
        scenario: Scenario::Errors,
    },
];

const SCENARIOS: &[Scenario] = &[
    Scenario::Transport,
    Scenario::Negotiation,
    Scenario::Authentication,
    Scenario::Sessions,
    Scenario::SessionContracts,
    Scenario::Streaming,
    Scenario::Cancellation,
    Scenario::Synchronization,
    Scenario::Errors,
];

pub(super) async fn run_suite_at(
    target: String,
    options: RunOptions,
    local_fixture: bool,
) -> Result<Report, ConformanceError> {
    validate_selected_cases(&options.selected_cases, local_fixture)?;
    let mut observed = HashMap::new();
    for scenario in SCENARIOS {
        let specs = EXTERNAL_CASES
            .iter()
            .filter(|spec| spec.scenario == *scenario)
            .collect::<Vec<_>>();
        if !specs
            .iter()
            .any(|spec| external_case_selected(&options.selected_cases, spec))
        {
            continue;
        }
        let run = match scenario {
            Scenario::Transport => basic::ping(&target, options.timeout).await,
            Scenario::Negotiation => basic::negotiation(&target, options.timeout).await,
            Scenario::Authentication => {
                authentication::run(&target, options.timeout, options.auth_token.as_deref()).await
            }
            Scenario::Sessions => {
                basic::sessions(&target, options.timeout, options.auth_token.as_deref()).await
            }
            Scenario::SessionContracts => {
                session_contracts::run(&target, options.timeout, options.auth_token.as_deref())
                    .await
            }
            Scenario::Streaming => {
                turns::streaming_lifecycle(
                    &target,
                    options.timeout,
                    options.model_checks,
                    options.auth_token.as_deref(),
                )
                .await
            }
            Scenario::Cancellation => {
                turns::cancellation(
                    &target,
                    options.timeout,
                    options.model_checks,
                    options.auth_token.as_deref(),
                )
                .await
            }
            Scenario::Synchronization => {
                advanced::synchronization_replay(
                    &target,
                    options.timeout,
                    options.model_checks,
                    options.auth_token.as_deref(),
                )
                .await
            }
            Scenario::Errors => advanced::errors(&target, options.timeout).await,
        };
        for check in run.checks {
            observed.insert(check.name, check);
        }
        for spec in specs {
            if external_case_selected(&options.selected_cases, spec)
                && !observed.contains_key(spec.name)
            {
                observed.insert(
                    spec.name,
                    ScenarioCheck::failed(
                        spec.name,
                        run.abort_detail
                            .as_deref()
                            .unwrap_or("scenario omitted this selected check"),
                    ),
                );
            }
        }
    }
    let mut cases = EXTERNAL_CASES
        .iter()
        .map(|spec| {
            if !external_case_selected(&options.selected_cases, spec) {
                return skipped(spec.name, spec.scope, "not selected");
            }
            match observed.remove(spec.name) {
                Some(check) => CaseResult {
                    name: spec.name,
                    status: check.status,
                    scope: spec.scope,
                    detail: check.detail,
                },
                None => CaseResult {
                    name: spec.name,
                    status: CaseStatus::Failed,
                    scope: spec.scope,
                    detail: "selected case was not executed".to_owned(),
                },
            }
        })
        .collect::<Vec<_>>();
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
        target: redact_target(&target),
        model_checks: options.model_checks,
        cases,
        format: options.format,
    })
}

pub(super) fn case_selected(selected: &[String], name: &str) -> bool {
    selected.is_empty() || selected.iter().any(|case| case == name)
}

fn external_case_selected(selected: &[String], spec: &CaseSpec) -> bool {
    selected.is_empty()
        || selected
            .iter()
            .any(|case| case == spec.name || case == spec.group)
}

pub(super) fn validate_selected_cases(
    selected: &[String],
    local_fixture: bool,
) -> Result<(), ConformanceError> {
    const STANDARD_GROUPS: &[&str] = &[
        "transport",
        "negotiation",
        "authentication",
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
        "chat-drafts",
        "draft-send-lifecycle",
        "response-part-ordering",
        "draft-model-only",
        "conversation-history",
    ];

    for name in selected {
        if STANDARD_GROUPS.contains(&name.as_str())
            || EXTERNAL_CASES.iter().any(|spec| spec.name == name)
        {
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
            EXTERNAL_CASES
                .iter()
                .map(|spec| spec.name)
                .chain(STANDARD_GROUPS.iter().copied())
                .collect::<Vec<_>>()
                .join(", "),
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
    use super::{external_case_selected, validate_selected_cases, EXTERNAL_CASES};

    #[test]
    fn selection_rejects_unknown_and_local_only_cases_on_external_hosts() {
        assert!(validate_selected_cases(&["nope".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["upstream-error".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["initialize-version".to_owned()], false).is_ok());
        assert!(validate_selected_cases(&["persistence-restart".to_owned()], true).is_ok());
        assert!(validate_selected_cases(&["workspace-validation".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["shell-approval-persistence".to_owned()], true).is_ok());
        assert!(validate_selected_cases(&["responses-streaming".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["api-auto-selection".to_owned()], true).is_ok());
        assert!(validate_selected_cases(&["model-catalog".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["session-model-restart".to_owned()], true).is_ok());
        assert!(validate_selected_cases(&["chat-drafts".to_owned()], false).is_err());
        assert!(validate_selected_cases(&["draft-send-lifecycle".to_owned()], true).is_ok());
        assert!(validate_selected_cases(&["response-part-ordering".to_owned()], true).is_ok());
        assert!(validate_selected_cases(&["draft-model-only".to_owned()], true).is_ok());
        assert!(validate_selected_cases(&["conversation-history".to_owned()], true).is_ok());
    }

    #[test]
    fn legacy_group_names_select_each_granular_case() {
        let selected = vec!["negotiation".to_owned()];
        let negotiation = EXTERNAL_CASES
            .iter()
            .filter(|spec| spec.group == "negotiation")
            .collect::<Vec<_>>();
        assert!(negotiation.len() > 1);
        assert!(negotiation
            .iter()
            .all(|spec| external_case_selected(&selected, spec)));
        assert!(EXTERNAL_CASES
            .iter()
            .filter(|spec| spec.group != "negotiation")
            .all(|spec| !external_case_selected(&selected, spec)));
    }

    #[test]
    fn external_case_names_are_unique() {
        let mut names = EXTERNAL_CASES
            .iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        let original_len = names.len();
        names.dedup();
        assert_eq!(names.len(), original_len);
    }
}
