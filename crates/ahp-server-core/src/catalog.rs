//! Shared projections for session catalogs and outstanding chat requests.

use ahp_types::actions::{
    SessionInputNeededRemovedAction, SessionInputNeededSetAction, StateAction,
};
use ahp_types::state::{
    ChatSummary, ResponsePart, SessionChatSummary, SessionInputRequest,
    SessionToolClientExecutionRequest, SessionToolConfirmationRequest, ToolCallConfirmationState,
    ToolCallContributor, ToolCallState,
};

/// Projects the complete, ordered lightweight root catalog.
#[must_use]
pub fn compact_chats(chats: &[ChatSummary]) -> Vec<SessionChatSummary> {
    chats
        .iter()
        .map(|chat| SessionChatSummary {
            resource: chat.resource.clone(),
            title: chat.title.clone(),
            origin: chat.origin.clone(),
            interactivity: chat.interactivity,
            status: Some(chat.status),
            changes: chat.changes.clone(),
        })
        .collect()
}

/// Mirrors tool confirmations and client-owned executions without requiring a chat subscription.
#[must_use]
pub fn chat_input_requests(
    chat: &str,
    turn_id: &str,
    parts: &[ResponsePart],
) -> Vec<SessionInputRequest> {
    parts
        .iter()
        .filter_map(|part| {
            let ResponsePart::ToolCall(part) = part else {
                return None;
            };
            let (tool_call_id, tool_call) = match &part.tool_call {
                ToolCallState::PendingConfirmation(call) => (
                    &call.tool_call_id,
                    ToolCallConfirmationState::PendingConfirmation(call.clone()),
                ),
                ToolCallState::PendingResultConfirmation(call) => (
                    &call.tool_call_id,
                    ToolCallConfirmationState::PendingResultConfirmation(call.clone()),
                ),
                ToolCallState::Running(call) => {
                    let Some(ToolCallContributor::Client(client)) = &call.contributor else {
                        return None;
                    };
                    return Some(SessionInputRequest::ToolClientExecution(
                        SessionToolClientExecutionRequest {
                            id: request_id(chat, turn_id, &call.tool_call_id),
                            chat: chat.into(),
                            turn_id: turn_id.into(),
                            client_id: client.client_id.clone(),
                            tool_call: call.clone(),
                        },
                    ));
                }
                _ => return None,
            };
            Some(SessionInputRequest::ToolConfirmation(
                SessionToolConfirmationRequest {
                    id: request_id(chat, turn_id, tool_call_id),
                    chat: chat.into(),
                    turn_id: turn_id.into(),
                    tool_call,
                },
            ))
        })
        .collect()
}

fn request_id(chat: &str, turn_id: &str, tool_call_id: &str) -> String {
    format!(
        "{}:{chat}{}:{turn_id}{}:{tool_call_id}",
        chat.len(),
        turn_id.len(),
        tool_call_id.len()
    )
}

fn id(request: &SessionInputRequest) -> &str {
    match request {
        SessionInputRequest::ToolConfirmation(request) => &request.id,
        SessionInputRequest::ToolClientExecution(request) => &request.id,
        _ => unreachable!("only derived tool requests are reconciled"),
    }
}

/// Produces replayable upserts and removals for a derived session input queue.
#[must_use]
pub fn input_request_actions(
    before: &[SessionInputRequest],
    after: &[SessionInputRequest],
) -> Vec<StateAction> {
    let mut actions = Vec::new();
    for previous in before {
        if !after.iter().any(|request| id(request) == id(previous)) {
            actions.push(StateAction::SessionInputNeededRemoved(
                SessionInputNeededRemovedAction {
                    id: id(previous).into(),
                },
            ));
        }
    }
    for request in after {
        if !before.iter().any(|previous| previous == request) {
            actions.push(StateAction::SessionInputNeededSet(Box::new(
                SessionInputNeededSetAction {
                    request: request.clone(),
                },
            )));
        }
    }
    actions
}

/// Reconciles derived entries while preserving the reducer's insertion order.
#[must_use]
pub fn updated_input_requests(
    before: &[SessionInputRequest],
    after: &[SessionInputRequest],
) -> Vec<SessionInputRequest> {
    let mut result = before
        .iter()
        .filter_map(|previous| {
            after
                .iter()
                .find(|request| id(request) == id(previous))
                .cloned()
        })
        .collect::<Vec<_>>();
    for request in after {
        if !result.iter().any(|previous| id(previous) == id(request)) {
            result.push(request.clone());
        }
    }
    result
}
