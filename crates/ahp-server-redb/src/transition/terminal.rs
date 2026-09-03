//! Terminal turn transitions.

use ahp_server_core::{CommittedTransition, FinishTurn, StoreConflict, StoreError};
use ahp_types::actions::{
    ChatErrorAction, ChatTurnCancelledAction, ChatTurnCompleteAction, StateAction,
};
use ahp_types::state::{ErrorResponsePart, ResponsePart};

use crate::replay::{Draft, TransitionWork};
use crate::sessions::write_chat;
use crate::types::{
    now_millis, now_rfc3339, terminalize_incomplete_tool_calls, trim_turn_history, PersistedTurn,
    PersistedTurnState,
};
use crate::RedbHostStore;

use super::{owned_chat, summary_draft};

impl RedbHostStore {
    pub(crate) fn finish_turn_transition(
        &self,
        request: FinishTurn,
    ) -> Result<CommittedTransition<i64>, StoreError> {
        let (chat_uri, turn_id, state, error, origin, model_cursor) = match request {
            FinishTurn::Complete {
                chat,
                turn_id,
                model_cursor,
            } => (
                chat,
                turn_id,
                PersistedTurnState::Complete,
                None,
                None,
                Some(model_cursor),
            ),
            FinishTurn::Cancelled {
                chat,
                turn_id,
                origin,
            } => (
                chat,
                turn_id,
                PersistedTurnState::Cancelled,
                None,
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
                PersistedTurnState::Error,
                Some(error),
                None,
                None,
            ),
        };
        self.commit_transition(move |write| {
            let mut chat = owned_chat(write, &chat_uri)?;
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
            let duration = now_millis().saturating_sub(active.started_millis);
            let mut parts = active.response_parts;
            if state != PersistedTurnState::Complete {
                terminalize_incomplete_tool_calls(&mut parts, "Tool call was interrupted.");
            }
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
            chat.turns.push(PersistedTurn {
                id: active.id,
                started_at: active.started_at,
                duration,
                message: active.message,
                response: active.response,
                response_parts: parts,
                state,
                error: error.clone(),
            });
            trim_turn_history(&mut chat.turns);
            chat.modified_at = now_rfc3339();
            write_chat(write, &chat)?;
            Ok(TransitionWork::new(
                duration,
                vec![
                    Draft::action(
                        chat_uri,
                        terminal_action(&turn_id, duration, state, error_part)?,
                        origin,
                    ),
                    summary_draft(&chat),
                ],
            ))
        })
    }
}

fn terminal_action(
    turn_id: &str,
    duration: i64,
    state: PersistedTurnState,
    error: Option<ErrorResponsePart>,
) -> Result<StateAction, StoreError> {
    Ok(match state {
        PersistedTurnState::Complete => StateAction::ChatTurnComplete(ChatTurnCompleteAction {
            turn_id: turn_id.into(),
            duration,
            meta: None,
        }),
        PersistedTurnState::Cancelled => StateAction::ChatTurnCancelled(ChatTurnCancelledAction {
            turn_id: turn_id.into(),
            duration,
            meta: None,
        }),
        PersistedTurnState::Error => StateAction::ChatError(ChatErrorAction {
            turn_id: turn_id.into(),
            duration,
            part: error.ok_or_else(|| {
                StoreError::Backend("error terminal state requires error information".into())
            })?,
            meta: None,
        }),
    })
}
