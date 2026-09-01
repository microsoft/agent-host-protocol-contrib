//! Restart recovery for durable active turns.

use ahp_server_core::StoreError;
use ahp_types::actions::{ChatErrorAction, StateAction};
use ahp_types::state::ErrorInfo;

use crate::replay::{Draft, TransitionWork};
use crate::sessions::{load_chat, session_owns_chat, write_chat};
use crate::transition::summary_draft;
use crate::types::{
    now_millis, now_rfc3339, terminalize_incomplete_tool_calls, trim_turn_history, PersistedTurn,
    PersistedTurnState,
};
use crate::RedbHostStore;

impl RedbHostStore {
    pub(crate) fn recover_incomplete_turns(&self) -> Result<(), StoreError> {
        for session_uri in self.session_ids()? {
            let Some(session) = self.get_session(&session_uri)? else {
                continue;
            };
            for chat_uri in session.chats {
                self.recover_chat(&chat_uri)?;
            }
        }
        Ok(())
    }

    fn recover_chat(&self, chat_uri: &str) -> Result<(), StoreError> {
        if self
            .get_chat(chat_uri)?
            .is_none_or(|chat| chat.active.is_none())
        {
            return Ok(());
        }
        let chat_uri = chat_uri.to_owned();
        self.commit_transition(move |write| {
            let mut chat = load_chat(write, &chat_uri)?.ok_or_else(|| {
                StoreError::Backend("active chat disappeared during restart recovery".into())
            })?;
            if !session_owns_chat(write, &chat)? {
                return Err(StoreError::Backend(
                    "active chat is not owned by its session during restart recovery".into(),
                ));
            }
            let active = chat.active.take().ok_or_else(|| {
                StoreError::Backend("active turn disappeared during restart recovery".into())
            })?;
            let duration = now_millis().saturating_sub(active.started_millis);
            let error = ErrorInfo {
                error_type: "host_restarted".into(),
                message: "the host restarted before this turn completed".into(),
                stack: None,
                meta: None,
            };
            let mut parts = active.response_parts;
            terminalize_incomplete_tool_calls(
                &mut parts,
                "Tool call was interrupted by host restart.",
            );
            chat.turns.push(PersistedTurn {
                id: active.id.clone(),
                started_at: active.started_at,
                duration,
                message: active.message,
                response: active.response,
                response_parts: parts,
                state: PersistedTurnState::Error,
                error: Some(error.clone()),
            });
            trim_turn_history(&mut chat.turns);
            chat.modified_at = now_rfc3339();
            write_chat(write, &chat)?;
            Ok(TransitionWork::new(
                (),
                vec![
                    Draft::action(
                        chat_uri,
                        StateAction::ChatError(ChatErrorAction {
                            turn_id: active.id,
                            duration,
                            error,
                            meta: None,
                        }),
                        None,
                    ),
                    summary_draft(&chat),
                ],
            ))
        })?;
        Ok(())
    }
}
