//! Typed root, session, and chat snapshot projections.

use crate::db::{backend, read_record, read_session_ids};
use crate::tables::{META, SERVER_SEQUENCE};
use crate::types::{
    active_turn, aggregate_chat_summary, chat_status, chat_summary, turn, PersistedChat,
    PersistedLifecycle, PersistedSession,
};
use crate::RedbHostStore;
use ahp_server_core::{SnapshotRequest, StoreConflict, StoreError};
use ahp_types::common::ROOT_RESOURCE_URI;
use ahp_types::state::{
    AgentCapabilities, AgentInfo, ChatState, ChatSummary, MultipleChatsCapability, RootState,
    SessionLifecycle, SessionState, SessionSummary, Snapshot, SnapshotState,
};

impl RedbHostStore {
    pub(crate) fn snapshot_for(&self, request: SnapshotRequest) -> Result<Snapshot, StoreError> {
        let database = self.database()?;
        let read = database.get()?.begin_read().map_err(backend)?;
        let from_seq = sequence(&read)?;
        if request.channel == ROOT_RESOURCE_URI {
            return root_snapshot(&read, request, from_seq);
        }
        if let Some(session) =
            read_record::<PersistedSession>(&read, crate::tables::SESSIONS, &request.channel)?
        {
            return session_snapshot(&read, request, from_seq, session);
        }
        let chat = read_record::<PersistedChat>(&read, crate::tables::CHATS, &request.channel)?
            .ok_or_else(|| StoreConflict::UnknownChannel(request.channel.clone()))?;
        Ok(Snapshot {
            resource: request.channel,
            from_seq,
            state: SnapshotState::Chat(Box::new(chat_state(&read, &chat)?)),
        })
    }

    pub(crate) fn session_summaries(&self) -> Result<Vec<SessionSummary>, StoreError> {
        let database = self.database()?;
        let read = database.get()?.begin_read().map_err(backend)?;
        let mut summaries = Vec::new();
        for session_uri in read_session_ids(&read)? {
            if let Some(session) =
                read_record::<PersistedSession>(&read, crate::tables::SESSIONS, &session_uri)?
            {
                summaries.push(session_summary(&read, &session)?);
            }
        }
        summaries.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
        Ok(summaries)
    }

    pub(crate) fn summary_for(&self, session_uri: &str) -> Result<SessionSummary, StoreError> {
        let database = self.database()?;
        let read = database.get()?.begin_read().map_err(backend)?;
        let session = read_record::<PersistedSession>(&read, crate::tables::SESSIONS, session_uri)?
            .ok_or_else(|| StoreConflict::UnknownChannel(session_uri.into()))?;
        session_summary(&read, &session)
    }
}

fn root_snapshot(
    read: &redb::ReadTransaction,
    request: SnapshotRequest,
    from_seq: i64,
) -> Result<Snapshot, StoreError> {
    let active_sessions = i64::try_from(read_session_ids(read)?.len()).unwrap_or(i64::MAX);
    Ok(Snapshot {
        resource: ROOT_RESOURCE_URI.into(),
        from_seq,
        state: SnapshotState::Root(Box::new(RootState {
            agents: vec![AgentInfo {
                provider: "openai".into(),
                display_name: "OpenAI-compatible".into(),
                description: "A streaming OpenAI-compatible agent.".into(),
                models: request.model_catalog.models.clone(),
                protected_resources: None,
                customizations: None,
                capabilities: Some(AgentCapabilities {
                    multiple_chats: Some(MultipleChatsCapability::default()),
                    multiple_working_directories: None,
                }),
            }],
            active_sessions: Some(active_sessions),
            terminals: None,
            config: None,
            meta: None,
        })),
    })
}

fn session_snapshot(
    read: &redb::ReadTransaction,
    request: SnapshotRequest,
    from_seq: i64,
    session: PersistedSession,
) -> Result<Snapshot, StoreError> {
    let summary = session_summary(read, &session)?;
    let chats = chat_summaries(read, &session.chats)?;
    let model_selection = session
        .model_selection
        .clone()
        .unwrap_or_else(|| request.model_catalog.default_selection.clone());
    Ok(Snapshot {
        resource: request.channel,
        from_seq,
        state: SnapshotState::Session(Box::new(SessionState {
            provider: session.provider,
            title: summary.title,
            status: summary.status,
            activity: summary.activity,
            project: None,
            working_directories: (!session.working_directories.is_empty())
                .then_some(session.working_directories),
            annotations: None,
            lifecycle: match session.lifecycle {
                PersistedLifecycle::Creating => SessionLifecycle::Creating,
                PersistedLifecycle::Ready => SessionLifecycle::Ready,
            },
            creation_error: None,
            server_tools: Some(request.server_tools),
            active_clients: Vec::new(),
            chats,
            default_chat: session.default_chat,
            config: Some(request.model_catalog.session_config_state(&model_selection)),
            customizations: None,
            changesets: None,
            input_needed: None,
            meta: None,
        })),
    })
}

fn session_summary(
    read: &redb::ReadTransaction,
    session: &PersistedSession,
) -> Result<SessionSummary, StoreError> {
    let chats = chat_summaries(read, &session.chats)?;
    let (status, activity, modified_at) =
        aggregate_chat_summary(&chats, session.default_chat.as_ref(), &session.created_at);
    Ok(SessionSummary {
        provider: session.provider.clone(),
        title: session.title.clone(),
        status,
        activity,
        project: None,
        working_directories: (!session.working_directories.is_empty())
            .then_some(session.working_directories.clone()),
        annotations: None,
        resource: session.uri.clone(),
        created_at: session.created_at.clone(),
        modified_at,
        changes: None,
        meta: None,
    })
}

fn chat_summaries(
    read: &redb::ReadTransaction,
    chat_uris: &[ahp_types::common::Uri],
) -> Result<Vec<ChatSummary>, StoreError> {
    chat_uris.iter().try_fold(Vec::new(), |mut summaries, uri| {
        if let Some(chat) = read_record::<PersistedChat>(read, crate::tables::CHATS, uri)? {
            summaries.push(chat_summary(&chat));
        }
        Ok(summaries)
    })
}

fn chat_state(read: &redb::ReadTransaction, chat: &PersistedChat) -> Result<ChatState, StoreError> {
    let working_directories =
        read_record::<PersistedSession>(read, crate::tables::SESSIONS, &chat.session_uri)?
            .and_then(|session| {
                (!session.working_directories.is_empty()).then_some(session.working_directories)
            });
    Ok(ChatState {
        resource: chat.uri.clone(),
        title: chat.title.clone(),
        status: chat_status(chat),
        activity: chat
            .active
            .as_ref()
            .map(|_| "Waiting for model response".into()),
        modified_at: chat.modified_at.clone(),
        origin: None,
        interactivity: None,
        working_directories,
        turns: chat.turns.iter().map(turn).collect(),
        turns_next_cursor: None,
        active_turn: chat.active.as_ref().map(active_turn),
        steering_message: None,
        queued_messages: None,
        draft: None,
        meta: None,
    })
}

fn sequence(read: &redb::ReadTransaction) -> Result<i64, StoreError> {
    let meta = read.open_table(META).map_err(backend)?;
    let value = meta
        .get(SERVER_SEQUENCE)
        .map_err(backend)?
        .ok_or_else(|| StoreError::Backend("server sequence metadata is missing".into()))?;
    value
        .value()
        .parse()
        .map_err(|error| StoreError::Backend(format!("invalid server sequence metadata: {error}")))
}
