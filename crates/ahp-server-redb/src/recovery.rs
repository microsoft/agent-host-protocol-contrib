//! Restart recovery for interrupted turns and disconnected active clients.

use ahp_server_core::StoreError;
use ahp_types::actions::{ChatErrorAction, SessionActiveClientRemovedAction, StateAction};
use ahp_types::state::{ErrorInfo, ErrorResponsePart, ResponsePart};

use crate::replay::{Draft, TransitionWork};
use crate::sessions::{load_chat, load_session, session_owns_chat, write_chat, write_session};
use crate::transition::summary_draft;
use crate::types::{
    now_millis, terminalize_incomplete_tool_calls, trim_turn_history, PersistedTurn,
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
            self.recover_active_clients(&session_uri)?;
        }
        Ok(())
    }

    fn recover_active_clients(&self, session_uri: &str) -> Result<(), StoreError> {
        if self
            .get_session(session_uri)?
            .is_none_or(|session| session.active_clients.is_empty())
        {
            return Ok(());
        }
        let session_uri = session_uri.to_owned();
        self.commit_transition(move |write| {
            Ok(TransitionWork::new(
                (),
                clear_active_clients(write, &session_uri)?,
            ))
        })?;
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
            let duration = now_millis().saturating_sub(active.started_millis).max(0);
            let modified_at = chrono::DateTime::parse_from_rfc3339(&active.started_at)
                .map_err(|error| StoreError::Backend(error.to_string()))?
                .checked_add_signed(chrono::TimeDelta::milliseconds(duration))
                .ok_or_else(|| StoreError::Backend("turn timestamp overflow".into()))?
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
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
            let error_part = ErrorResponsePart {
                error: error.clone(),
                resumable: None,
            };
            parts.push(ResponsePart::Error(error_part.clone()));
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
            chat.modified_at = modified_at;
            write_chat(write, &chat)?;
            let mut drafts = vec![
                Draft::action(
                    chat_uri,
                    StateAction::ChatError(ChatErrorAction {
                        turn_id: active.id,
                        duration,
                        part: error_part,
                        meta: None,
                    }),
                    None,
                ),
                summary_draft(&chat),
            ];
            drafts.extend(clear_active_clients(write, &chat.session_uri)?);
            Ok(TransitionWork::new((), drafts))
        })?;
        Ok(())
    }
}

fn clear_active_clients(
    write: &redb::WriteTransaction,
    session_uri: &str,
) -> Result<Vec<Draft>, StoreError> {
    let mut session = load_session(write, session_uri)?
        .ok_or_else(|| StoreError::Backend("session disappeared during restart recovery".into()))?;
    let clients = std::mem::take(&mut session.active_clients);
    if clients.is_empty() {
        return Ok(Vec::new());
    }
    write_session(write, &session)?;
    Ok(clients
        .into_iter()
        .map(|client| {
            Draft::action(
                session_uri,
                StateAction::SessionActiveClientRemoved(SessionActiveClientRemovedAction {
                    client_id: client.client_id,
                }),
                None,
            )
        })
        .collect())
}
