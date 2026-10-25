//! Atomic active-turn terminalization.

use ahp_types::actions::{
    ChatErrorAction, ChatTurnCancelledAction, ChatTurnCompleteAction, StateAction,
};
use ahp_types::state::{ErrorResponsePart, ResponsePart, ToolCallState, TurnState};
use chrono::{DateTime, SecondsFormat, Utc};

use super::{Data, StoredTurn};
use crate::memory::transitions::summary_draft;
use crate::memory::Draft;
use crate::store::{FinishTurn, StoreConflict, StoreError};

impl Data {
    pub(in crate::memory) fn finish_turn(
        &mut self,
        request: FinishTurn,
    ) -> Result<(i64, Vec<Draft>), StoreError> {
        let (chat_uri, turn_id, terminal, origin, model_cursor) = match request {
            FinishTurn::Complete {
                chat,
                turn_id,
                model_cursor,
            } => (chat, turn_id, None, None, Some(model_cursor)),
            FinishTurn::Cancelled {
                chat,
                turn_id,
                origin,
            } => (
                chat,
                turn_id,
                Some((TurnState::Cancelled, None)),
                origin,
                None,
            ),
            FinishTurn::Error {
                chat,
                turn_id,
                error,
            } => (
                chat,
                turn_id,
                Some((TurnState::Error, Some(error))),
                None,
                None,
            ),
        };
        let chat = self.chat_mut(&chat_uri)?;
        let active = chat
            .active
            .take()
            .ok_or_else(|| StoreConflict::NoActiveTurn(chat_uri.clone()))?;
        if active.id != turn_id {
            chat.active = Some(active);
            return Err(StoreConflict::TurnMismatch {
                chat: chat_uri,
                turn: turn_id,
            }
            .into());
        }
        let duration = Utc::now()
            .timestamp_millis()
            .saturating_sub(active.started_millis)
            .max(0);
        let modified_at = DateTime::parse_from_rfc3339(&active.started_at)
            .map_err(|error| StoreError::Backend(error.to_string()))?
            .checked_add_signed(chrono::TimeDelta::milliseconds(duration))
            .ok_or_else(|| StoreError::Backend("turn timestamp overflow".into()))?
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let (state, error) = terminal.unwrap_or((TurnState::Complete, None));
        let mut parts = active.response_parts;
        terminalize_tools(&mut parts)?;
        let error_part = error.clone().map(|error| ErrorResponsePart {
            error,
            resumable: None,
        });
        if let Some(part) = error_part.clone() {
            parts.push(ResponsePart::Error(part));
        }
        if let Some(model_cursor) = model_cursor {
            chat.model_cursor = model_cursor;
        }
        chat.turns.push(StoredTurn {
            id: active.id,
            started_at: active.started_at,
            duration,
            message: active.message,
            response_parts: parts,
            state,
            error: error.clone(),
        });
        if chat.turns.len() > 256 {
            chat.turns.remove(0);
        }
        chat.modified_at = modified_at;
        let action = terminal_action(&turn_id, duration, state, error_part)?;
        let summary = self.chat_summary(self.chat(&chat_uri)?);
        Ok((
            duration,
            vec![
                Draft::action(chat_uri.clone(), action, origin),
                summary_draft(self, &chat_uri, summary),
            ],
        ))
    }
}

fn terminal_action(
    turn_id: &str,
    duration: i64,
    state: TurnState,
    error: Option<ErrorResponsePart>,
) -> Result<StateAction, StoreError> {
    Ok(match state {
        TurnState::Complete => StateAction::ChatTurnComplete(ChatTurnCompleteAction {
            turn_id: turn_id.into(),
            duration,
            meta: None,
        }),
        TurnState::Cancelled => StateAction::ChatTurnCancelled(ChatTurnCancelledAction {
            turn_id: turn_id.into(),
            duration,
            meta: None,
        }),
        TurnState::Error => StateAction::ChatError(ChatErrorAction {
            turn_id: turn_id.into(),
            duration,
            part: error.ok_or_else(|| {
                StoreError::Backend("error terminal state requires error info".into())
            })?,
            meta: None,
        }),
    })
}

fn terminalize_tools(parts: &mut [ResponsePart]) -> Result<(), StoreError> {
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
        let streaming = matches!(&tool.tool_call, ToolCallState::Streaming(_));
        let mut value = serde_json::to_value(&tool.tool_call)
            .map_err(|error| StoreError::Backend(error.to_string()))?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| StoreError::Backend("tool call must be an object".into()))?;
        object.retain(|key, _| {
            matches!(
                key.as_str(),
                "toolCallId"
                    | "toolName"
                    | "displayName"
                    | "intention"
                    | "contributor"
                    | "_meta"
                    | "invocationMessage"
                    | "toolInput"
            )
        });
        if streaming {
            object.remove("toolInput");
        }
        object
            .entry("invocationMessage")
            .or_insert_with(|| serde_json::Value::String(String::new()));
        object.insert(
            "status".into(),
            serde_json::Value::String("cancelled".into()),
        );
        object.insert("reason".into(), serde_json::Value::String("skipped".into()));
        tool.tool_call = serde_json::from_value(value)
            .map_err(|error| StoreError::Backend(error.to_string()))?;
    }
    Ok(())
}
