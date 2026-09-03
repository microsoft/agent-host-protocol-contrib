//! Atomic active-turn terminalization.

use ahp_types::actions::{
    ChatErrorAction, ChatTurnCancelledAction, ChatTurnCompleteAction, StateAction,
};
use ahp_types::state::{ResponsePart, ToolCallState, TurnState};
use chrono::Utc;

use super::{now, tool_state_id, Data, StoredTurn};
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
            .saturating_sub(active.started_millis);
        let (state, error) = terminal.unwrap_or((TurnState::Complete, None));
        let mut parts = active.response_parts;
        if state != TurnState::Complete {
            terminalize_tools(&mut parts)?;
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
        chat.modified_at = now();
        let action = terminal_action(&turn_id, duration, state, error)?;
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
    error: Option<ahp_types::state::ErrorInfo>,
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
            error: error.ok_or_else(|| {
                StoreError::Backend("error terminal state requires error info".into())
            })?,
            meta: None,
        }),
    })
}

fn terminalize_tools(parts: &mut [ResponsePart]) -> Result<(), StoreError> {
    for part in parts.iter_mut() {
        let ResponsePart::ToolCall(tool) = part else {
            continue;
        };
        if matches!(
            tool.tool_call,
            ToolCallState::Streaming(_)
                | ToolCallState::PendingConfirmation(_)
                | ToolCallState::Running(_)
        ) {
            let value = serde_json::json!({
                "status": "cancelled",
                "toolCallId": tool_state_id(&tool.tool_call),
                "toolName": tool_name(&tool.tool_call),
                "displayName": tool_display(&tool.tool_call),
                "invocationMessage": "Tool call was interrupted.",
                "reason": "skipped",
                "reasonMessage": "Tool call was interrupted."
            });
            tool.tool_call = serde_json::from_value(value)
                .map_err(|error| StoreError::Backend(error.to_string()))?;
        }
    }
    Ok(())
}

fn tool_name(state: &ToolCallState) -> String {
    match state {
        ToolCallState::Streaming(item) => item.tool_name.clone(),
        ToolCallState::PendingConfirmation(item) => item.tool_name.clone(),
        ToolCallState::Running(item) => item.tool_name.clone(),
        ToolCallState::AuthRequired(item) => item.tool_name.clone(),
        ToolCallState::PendingResultConfirmation(item) => item.tool_name.clone(),
        ToolCallState::Completed(item) => item.tool_name.clone(),
        ToolCallState::Cancelled(item) => item.tool_name.clone(),
        ToolCallState::Unknown(_) => String::new(),
    }
}

fn tool_display(state: &ToolCallState) -> String {
    match state {
        ToolCallState::Streaming(item) => item.display_name.clone(),
        ToolCallState::PendingConfirmation(item) => item.display_name.clone(),
        ToolCallState::Running(item) => item.display_name.clone(),
        ToolCallState::AuthRequired(item) => item.display_name.clone(),
        ToolCallState::PendingResultConfirmation(item) => item.display_name.clone(),
        ToolCallState::Completed(item) => item.display_name.clone(),
        ToolCallState::Cancelled(item) => item.display_name.clone(),
        ToolCallState::Unknown(_) => String::new(),
    }
}
