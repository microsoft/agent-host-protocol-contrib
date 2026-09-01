//! Atomic session and chat catalogue transitions.

use crate::db::{
    backend, read_record_from_write, read_session_ids_from_write, write_record, write_session_ids,
};
use crate::replay::{Draft, TransitionWork};
use crate::tables::{CHATS, SESSIONS};
use crate::types::{
    chat_summary, now_rfc3339, PersistedChat, PersistedLifecycle, PersistedSession,
};
use crate::RedbHostStore;
use ahp_server_core::{
    CommittedTransition, CreateChat, CreateSession, CreatedSession, DisposedSession,
    MarkSessionReady, StoreConflict, StoreError,
};
use ahp_types::actions::{
    RootActiveSessionsChangedAction, SessionChatAddedAction, SessionReadyAction, StateAction,
};
use ahp_types::common::ROOT_RESOURCE_URI;

impl RedbHostStore {
    pub(crate) fn create_session_transition(
        &self,
        request: CreateSession,
    ) -> Result<CommittedTransition<CreatedSession>, StoreError> {
        self.commit_transition(move |write| {
            if load_session(write, &request.session)?.is_some() {
                return Err(StoreConflict::SessionExists(request.session).into());
            }
            if load_chat(write, &request.default_chat)?.is_some() {
                return Err(StoreConflict::ChatExists(request.default_chat).into());
            }
            let timestamp = now_rfc3339();
            let chat = PersistedChat {
                uri: request.default_chat.clone(),
                session_uri: request.session.clone(),
                title: "New chat".into(),
                created_at: timestamp.clone(),
                modified_at: timestamp.clone(),
                turns: Vec::new(),
                active: None,
            };
            let session = PersistedSession {
                uri: request.session.clone(),
                provider: request.provider,
                title: "New OpenAI-compatible session".into(),
                created_at: timestamp,
                working_directories: vec![request.workspace.clone()],
                lifecycle: PersistedLifecycle::Creating,
                chats: vec![request.default_chat],
                default_chat: Some(chat.uri.clone()),
            };
            let mut session_ids = read_session_ids_from_write(write)?;
            session_ids.push(session.uri.clone());
            write_record(write, SESSIONS, &session.uri, &session)?;
            write_record(write, CHATS, &chat.uri, &chat)?;
            write_session_ids(write, &session_ids)?;
            let summary = chat_summary(&chat);
            let count = i64::try_from(session_ids.len()).unwrap_or(i64::MAX);
            Ok(TransitionWork::new(
                CreatedSession {
                    session: session.uri,
                    default_chat: summary.clone(),
                    workspace: request.workspace,
                },
                vec![
                    Draft::action(
                        chat.session_uri,
                        StateAction::SessionChatAdded(SessionChatAddedAction { summary }),
                        None,
                    ),
                    Draft::action(
                        ROOT_RESOURCE_URI,
                        StateAction::RootActiveSessionsChanged(RootActiveSessionsChangedAction {
                            active_sessions: count,
                        }),
                        None,
                    ),
                ],
            ))
        })
    }

    pub(crate) fn mark_session_ready_transition(
        &self,
        request: MarkSessionReady,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.commit_transition(move |write| {
            let mut session = load_session(write, &request.session)?
                .ok_or_else(|| StoreConflict::UnknownChannel(request.session.clone()))?;
            if session.lifecycle == PersistedLifecycle::Ready {
                return Err(StoreError::Backend(format!(
                    "session {:?} is already ready",
                    request.session
                )));
            }
            session.lifecycle = PersistedLifecycle::Ready;
            write_session(write, &session)?;
            Ok(TransitionWork::new(
                (),
                vec![Draft::action(
                    request.session,
                    StateAction::SessionReady(SessionReadyAction {}),
                    None,
                )],
            ))
        })
    }

    pub(crate) fn create_chat_transition(
        &self,
        request: CreateChat,
    ) -> Result<CommittedTransition<ahp_types::state::ChatSummary>, StoreError> {
        self.commit_transition(move |write| {
            let mut session = load_session(write, &request.session)?
                .ok_or_else(|| StoreConflict::UnknownChannel(request.session.clone()))?;
            if session.lifecycle != PersistedLifecycle::Ready {
                return Err(StoreConflict::SessionNotReady(request.session).into());
            }
            if load_chat(write, &request.chat)?.is_some() {
                return Err(StoreConflict::ChatExists(request.chat).into());
            }
            let timestamp = now_rfc3339();
            let chat = PersistedChat {
                uri: request.chat.clone(),
                session_uri: session.uri.clone(),
                title: "New chat".into(),
                created_at: timestamp.clone(),
                modified_at: timestamp,
                turns: Vec::new(),
                active: None,
            };
            session.chats.push(chat.uri.clone());
            write_chat(write, &chat)?;
            write_session(write, &session)?;
            let summary = chat_summary(&chat);
            Ok(TransitionWork::new(
                summary.clone(),
                vec![Draft::action(
                    request.session,
                    StateAction::SessionChatAdded(SessionChatAddedAction { summary }),
                    None,
                )],
            ))
        })
    }

    pub(crate) fn dispose_session_transition(
        &self,
        session_uri: &str,
    ) -> Result<CommittedTransition<DisposedSession>, StoreError> {
        let session_uri = session_uri.to_owned();
        self.commit_transition(move |write| {
            let session = load_session(write, &session_uri)?
                .ok_or_else(|| StoreConflict::UnknownChannel(session_uri.clone()))?;
            let mut session_ids = read_session_ids_from_write(write)?;
            session_ids.retain(|item| item != &session_uri);
            let mut chats = write.open_table(CHATS).map_err(backend)?;
            for chat in &session.chats {
                chats.remove(chat.as_str()).map_err(backend)?;
            }
            drop(chats);
            let mut sessions = write.open_table(SESSIONS).map_err(backend)?;
            sessions.remove(session_uri.as_str()).map_err(backend)?;
            drop(sessions);
            write_session_ids(write, &session_ids)?;
            let count = i64::try_from(session_ids.len()).unwrap_or(i64::MAX);
            Ok(TransitionWork::new(
                DisposedSession {
                    session: session_uri,
                    chats: session.chats,
                },
                vec![Draft::action(
                    ROOT_RESOURCE_URI,
                    StateAction::RootActiveSessionsChanged(RootActiveSessionsChangedAction {
                        active_sessions: count,
                    }),
                    None,
                )],
            ))
        })
    }
}

pub(crate) fn load_session(
    write: &redb::WriteTransaction,
    uri: &str,
) -> Result<Option<PersistedSession>, StoreError> {
    read_record_from_write(write, SESSIONS, uri)
}

pub(crate) fn load_chat(
    write: &redb::WriteTransaction,
    uri: &str,
) -> Result<Option<PersistedChat>, StoreError> {
    read_record_from_write(write, CHATS, uri)
}

pub(crate) fn write_session(
    write: &redb::WriteTransaction,
    session: &PersistedSession,
) -> Result<(), StoreError> {
    write_record(write, SESSIONS, &session.uri, session)
}

pub(crate) fn write_chat(
    write: &redb::WriteTransaction,
    chat: &PersistedChat,
) -> Result<(), StoreError> {
    write_record(write, CHATS, &chat.uri, chat)
}

pub(crate) fn session_owns_chat(
    write: &redb::WriteTransaction,
    chat: &PersistedChat,
) -> Result<bool, StoreError> {
    Ok(load_session(write, &chat.session_uri)?
        .is_some_and(|session| session.chats.contains(&chat.uri)))
}
