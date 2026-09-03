//! Persisted records and conversions to AHP state values.

use ahp_server_core::ModelCursor;
use ahp_types::common::Uri;
use ahp_types::state::{
    ActiveTurn, ChatSummary, ErrorInfo, MarkdownResponsePart, Message, ModelSelection,
    ResponsePart, SessionStatus, ToolCallState, Turn, TurnState,
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::tables::TURN_HISTORY_LIMIT;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PersistedSession {
    pub(crate) uri: Uri,
    pub(crate) provider: String,
    pub(crate) title: String,
    pub(crate) created_at: String,
    #[serde(default)]
    pub(crate) working_directories: Vec<Uri>,
    pub(crate) lifecycle: PersistedLifecycle,
    #[serde(default)]
    pub(crate) chats: Vec<Uri>,
    #[serde(default)]
    pub(crate) default_chat: Option<Uri>,
    #[serde(default)]
    pub(crate) model_selection: Option<ModelSelection>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum PersistedLifecycle {
    Creating,
    Ready,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PersistedChat {
    pub(crate) uri: Uri,
    pub(crate) session_uri: Uri,
    pub(crate) title: String,
    pub(crate) created_at: String,
    pub(crate) modified_at: String,
    #[serde(default)]
    pub(crate) turns: Vec<PersistedTurn>,
    #[serde(default)]
    pub(crate) active: Option<ActiveRecord>,
    #[serde(default)]
    pub(crate) draft: Option<Message>,
    #[serde(default)]
    pub(crate) model_cursor: Option<ModelCursor>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ActiveRecord {
    pub(crate) id: String,
    pub(crate) started_at: String,
    pub(crate) started_millis: i64,
    pub(crate) message: Message,
    #[serde(default)]
    pub(crate) response: String,
    #[serde(default)]
    pub(crate) response_parts: Vec<ResponsePart>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PersistedTurn {
    pub(crate) id: String,
    pub(crate) started_at: String,
    pub(crate) duration: i64,
    pub(crate) message: Message,
    #[serde(default)]
    pub(crate) response: String,
    #[serde(default)]
    pub(crate) response_parts: Vec<ResponsePart>,
    pub(crate) state: PersistedTurnState,
    pub(crate) error: Option<ErrorInfo>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum PersistedTurnState {
    Complete,
    Cancelled,
    Error,
}

pub(crate) fn chat_summary(chat: &PersistedChat) -> ChatSummary {
    ChatSummary {
        resource: chat.uri.clone(),
        title: chat.title.clone(),
        status: chat_status(chat),
        activity: chat
            .active
            .as_ref()
            .map(|_| "Waiting for model response".to_owned()),
        modified_at: chat.modified_at.clone(),
        origin: None,
        interactivity: None,
        working_directories: None,
    }
}

pub(crate) fn chat_status(chat: &PersistedChat) -> u32 {
    if chat.active.as_ref().is_some_and(active_turn_needs_input) {
        return SessionStatus::InputNeeded.bits();
    }
    if chat.active.is_some() {
        return SessionStatus::InProgress.bits();
    }
    match chat.turns.last().map(|turn| turn.state) {
        Some(PersistedTurnState::Error) => SessionStatus::Error.bits(),
        Some(PersistedTurnState::Complete | PersistedTurnState::Cancelled) | None => {
            SessionStatus::Idle.bits()
        }
    }
}

pub(crate) fn aggregate_chat_summary(
    chats: &[ChatSummary],
    default_chat: Option<&Uri>,
    fallback_modified_at: &str,
) -> (u32, Option<String>, String) {
    let Some(latest) = chats
        .iter()
        .max_by(|left, right| left.modified_at.cmp(&right.modified_at))
    else {
        return (
            SessionStatus::Idle.bits(),
            None,
            fallback_modified_at.to_owned(),
        );
    };
    let default = default_chat
        .and_then(|uri| chats.iter().find(|chat| chat.resource == *uri))
        .unwrap_or(latest);
    let driving_chat = chats
        .iter()
        .find(|chat| status_contains(chat.status, SessionStatus::InputNeeded))
        .or_else(|| {
            chats
                .iter()
                .find(|chat| status_contains(chat.status, SessionStatus::Error))
        })
        .unwrap_or(default);
    (
        driving_chat.status,
        driving_chat.activity.clone(),
        latest.modified_at.clone(),
    )
}

fn active_turn_needs_input(active: &ActiveRecord) -> bool {
    active.response_parts.iter().any(|part| {
        matches!(
            part,
            ResponsePart::ToolCall(tool)
                if matches!(
                    &tool.tool_call,
                    ToolCallState::PendingConfirmation(_)
                        | ToolCallState::PendingResultConfirmation(_)
                )
        )
    })
}

fn status_contains(status: u32, expected: SessionStatus) -> bool {
    SessionStatus::from_bits(status).contains(expected)
}

pub(crate) fn active_turn(active: &ActiveRecord) -> ActiveTurn {
    ActiveTurn {
        id: active.id.clone(),
        started_at: active.started_at.clone(),
        message: active.message.clone(),
        response_parts: response_parts(&active.id, &active.response, &active.response_parts),
        usage: None,
    }
}

pub(crate) fn turn(turn: &PersistedTurn) -> Turn {
    Turn {
        id: turn.id.clone(),
        started_at: Some(turn.started_at.clone()),
        duration: Some(turn.duration),
        message: turn.message.clone(),
        response_parts: response_parts(&turn.id, &turn.response, &turn.response_parts),
        usage: None,
        state: match turn.state {
            PersistedTurnState::Complete => TurnState::Complete,
            PersistedTurnState::Cancelled => TurnState::Cancelled,
            PersistedTurnState::Error => TurnState::Error,
        },
        error: turn.error.clone(),
    }
}

pub(crate) fn response_parts(
    turn_id: &str,
    response: &str,
    persisted: &[ResponsePart],
) -> Vec<ResponsePart> {
    if persisted.is_empty() {
        vec![ResponsePart::Markdown(MarkdownResponsePart {
            id: response_part_id(turn_id),
            content: response.to_owned(),
        })]
    } else {
        persisted.to_vec()
    }
}

pub(crate) fn terminalize_incomplete_tool_calls(parts: &mut [ResponsePart], message: &str) {
    for part in parts {
        let ResponsePart::ToolCall(tool) = part else {
            continue;
        };
        if matches!(
            &tool.tool_call,
            ToolCallState::Completed(_) | ToolCallState::Cancelled(_)
        ) {
            continue;
        }
        let Ok(mut state) = serde_json::to_value(&tool.tool_call) else {
            continue;
        };
        let Some(object) = state.as_object_mut() else {
            continue;
        };
        object.insert(
            "status".into(),
            serde_json::Value::String("cancelled".into()),
        );
        object.insert("reason".into(), serde_json::Value::String("skipped".into()));
        object.insert(
            "reasonMessage".into(),
            serde_json::Value::String(message.to_owned()),
        );
        object
            .entry(String::from("invocationMessage"))
            .or_insert_with(|| serde_json::Value::String(message.to_owned()));
        if let Ok(terminal) = serde_json::from_value(state) {
            tool.tool_call = terminal;
        }
    }
}

pub(crate) fn response_part_id(turn_id: &str) -> String {
    format!("response-{turn_id}")
}

pub(crate) fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

pub(crate) fn trim_turn_history(turns: &mut Vec<PersistedTurn>) {
    let excess = turns.len().saturating_sub(TURN_HISTORY_LIMIT);
    if excess > 0 {
        turns.drain(..excess);
    }
}

pub(crate) fn tool_call_id(state: &ToolCallState) -> &str {
    match state {
        ToolCallState::Streaming(item) => &item.tool_call_id,
        ToolCallState::PendingConfirmation(item) => &item.tool_call_id,
        ToolCallState::Running(item) => &item.tool_call_id,
        ToolCallState::AuthRequired(item) => &item.tool_call_id,
        ToolCallState::PendingResultConfirmation(item) => &item.tool_call_id,
        ToolCallState::Completed(item) => &item.tool_call_id,
        ToolCallState::Cancelled(item) => &item.tool_call_id,
        ToolCallState::Unknown(_) => "",
    }
}

pub(crate) fn part_tool_call_id(part: &ResponsePart) -> Option<&str> {
    if let ResponsePart::ToolCall(tool) = part {
        Some(tool_call_id(&tool.tool_call))
    } else {
        None
    }
}

pub(crate) fn valid_tool_transition(current: &ToolCallState, next: &ToolCallState) -> bool {
    matches!(
        (current, next),
        (
            ToolCallState::Streaming(_),
            ToolCallState::PendingConfirmation(_) | ToolCallState::Running(_)
        ) | (
            ToolCallState::PendingConfirmation(_),
            ToolCallState::Running(_) | ToolCallState::Cancelled(_)
        ) | (ToolCallState::Running(_), ToolCallState::Completed(_))
    )
}
