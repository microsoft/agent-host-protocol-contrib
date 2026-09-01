//! Atomic session and chat catalogue transitions.

use ahp_types::actions::{
    RootActiveSessionsChangedAction, SessionChatAddedAction, SessionReadyAction, StateAction,
};
use ahp_types::common::ROOT_RESOURCE_URI;

use super::state::{now, Chat, Session};
use super::{Draft, MemoryHostStore};
use crate::store::{
    CommittedTransition, CreateChat, CreateSession, CreatedSession, DisposedSession,
    MarkSessionReady, StoreConflict, StoreError,
};

pub(super) fn create_session(
    store: &MemoryHostStore,
    request: CreateSession,
) -> Result<CommittedTransition<CreatedSession>, StoreError> {
    store.transact(move |data| {
        if data.sessions.contains_key(&request.session) {
            return Err(StoreConflict::SessionExists(request.session).into());
        }
        if data.chats.contains_key(&request.default_chat) {
            return Err(StoreConflict::ChatExists(request.default_chat).into());
        }
        let timestamp = now();
        let chat = Chat::new(
            request.default_chat.clone(),
            request.session.clone(),
            timestamp.clone(),
        );
        let summary = data.chat_summary(&chat);
        data.chats.insert(chat.uri.clone(), chat);
        data.sessions.insert(
            request.session.clone(),
            Session::new(
                request.session.clone(),
                request.provider,
                request.workspace.clone(),
                timestamp,
            ),
        );
        data.session_mut(&request.session)?
            .chats
            .push(request.default_chat);
        let value = CreatedSession {
            session: request.session.clone(),
            default_chat: summary.clone(),
            workspace: request.workspace,
        };
        Ok((
            value,
            vec![
                Draft::action(
                    request.session,
                    StateAction::SessionChatAdded(SessionChatAddedAction { summary }),
                    None,
                ),
                Draft::action(
                    ROOT_RESOURCE_URI.into(),
                    StateAction::RootActiveSessionsChanged(RootActiveSessionsChangedAction {
                        active_sessions: i64::try_from(data.sessions.len()).unwrap_or(i64::MAX),
                    }),
                    None,
                ),
            ],
        ))
    })
}

pub(super) fn mark_session_ready(
    store: &MemoryHostStore,
    request: MarkSessionReady,
) -> Result<CommittedTransition<()>, StoreError> {
    store.transact(move |data| {
        data.session_mut(&request.session)?.ready = true;
        Ok((
            (),
            vec![Draft::action(
                request.session,
                StateAction::SessionReady(SessionReadyAction {}),
                None,
            )],
        ))
    })
}

pub(super) fn create_chat(
    store: &MemoryHostStore,
    request: CreateChat,
) -> Result<CommittedTransition<ahp_types::state::ChatSummary>, StoreError> {
    store.transact(move |data| {
        if data.chats.contains_key(&request.chat) {
            return Err(StoreConflict::ChatExists(request.chat).into());
        }
        if !data.session(&request.session)?.ready {
            return Err(StoreConflict::SessionNotReady(request.session).into());
        }
        let chat = Chat::new(request.chat.clone(), request.session.clone(), now());
        let summary = data.chat_summary(&chat);
        data.chats.insert(chat.uri.clone(), chat);
        data.session_mut(&request.session)?.chats.push(request.chat);
        Ok((
            summary.clone(),
            vec![Draft::action(
                request.session,
                StateAction::SessionChatAdded(SessionChatAddedAction { summary }),
                None,
            )],
        ))
    })
}

pub(super) fn dispose_session(
    store: &MemoryHostStore,
    session: &str,
) -> Result<CommittedTransition<DisposedSession>, StoreError> {
    let session = session.to_owned();
    store.transact(move |data| {
        let session_state = data
            .sessions
            .remove(&session)
            .ok_or_else(|| StoreConflict::UnknownChannel(session.clone()))?;
        for chat in &session_state.chats {
            data.chats.remove(chat);
        }
        let value = DisposedSession {
            session: session.clone(),
            chats: session_state.chats,
        };
        Ok((
            value,
            vec![Draft::action(
                ROOT_RESOURCE_URI.into(),
                StateAction::RootActiveSessionsChanged(RootActiveSessionsChangedAction {
                    active_sessions: i64::try_from(data.sessions.len()).unwrap_or(i64::MAX),
                }),
                None,
            )],
        ))
    })
}
