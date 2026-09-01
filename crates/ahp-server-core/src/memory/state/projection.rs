//! Typed root, session, and chat projections.

use ahp_types::common::ROOT_RESOURCE_URI;
use ahp_types::state::{
    ActiveTurn, AgentCapabilities, AgentInfo, ChatState, ChatSummary, MultipleChatsCapability,
    RootState, SessionLifecycle, SessionModelInfo, SessionState, SessionStatus, SessionSummary,
    Snapshot, SnapshotState, Turn, TurnState,
};

use super::{Active, Chat, Data, Session, StoredTurn};
use crate::store::{SnapshotRequest, StoreConflict, StoreError};

impl Data {
    pub(in crate::memory) fn chat_summary(&self, chat: &Chat) -> ChatSummary {
        ChatSummary {
            resource: chat.uri.clone(),
            title: "New chat".into(),
            status: chat_status(chat),
            activity: chat_activity(chat),
            modified_at: chat.modified_at.clone(),
            origin: None,
            interactivity: None,
            working_directories: None,
        }
    }

    pub(in crate::memory) fn session_summary(&self, session: &Session) -> SessionSummary {
        let chats = session
            .chats
            .iter()
            .filter_map(|id| self.chats.get(id))
            .collect::<Vec<_>>();
        let modified_at = chats
            .iter()
            .map(|chat| chat.modified_at.as_str())
            .max()
            .unwrap_or(&session.created_at)
            .to_owned();
        let primary = session
            .chats
            .first()
            .and_then(|id| self.chats.get(id))
            .or_else(|| chats.iter().max_by_key(|chat| &chat.modified_at).copied());
        let primary_status = primary.map_or(SessionStatus::Idle.bits(), chat_status);
        let needs_input = chats.iter().any(|chat| {
            SessionStatus::from_bits(chat_status(chat)).contains(SessionStatus::InputNeeded)
        });
        let has_error = chats
            .iter()
            .any(|chat| SessionStatus::from_bits(chat_status(chat)).contains(SessionStatus::Error));
        let status = match (needs_input, has_error) {
            (true, true) => SessionStatus::InputNeeded.bits() | SessionStatus::Error.bits(),
            (true, false) => SessionStatus::InputNeeded.bits(),
            (false, true) => SessionStatus::Error.bits(),
            (false, false) => primary_status,
        };
        SessionSummary {
            provider: session.provider.clone(),
            title: "New OpenAI-compatible session".into(),
            status,
            activity: needs_input
                .then(|| "Waiting for input".into())
                .or_else(|| primary.and_then(chat_activity)),
            project: None,
            working_directories: Some(vec![session.workspace.clone()]),
            annotations: None,
            resource: session.uri.clone(),
            created_at: session.created_at.clone(),
            modified_at,
            changes: None,
            meta: None,
        }
    }

    pub(in crate::memory) fn snapshot(
        &self,
        request: SnapshotRequest,
    ) -> Result<Snapshot, StoreError> {
        let from_seq = i64::try_from(self.server_seq)
            .map_err(|_| StoreError::Backend("server sequence overflow".into()))?;
        if request.channel == ROOT_RESOURCE_URI {
            return Ok(root_snapshot(self, request, from_seq));
        }
        if let Some(session) = self.sessions.get(&request.channel) {
            return Ok(session_snapshot(self, request, from_seq, session));
        }
        let chat = self
            .chats
            .get(&request.channel)
            .ok_or_else(|| StoreConflict::UnknownChannel(request.channel.clone()))?;
        Ok(Snapshot {
            resource: request.channel,
            from_seq,
            state: SnapshotState::Chat(Box::new(chat_state(chat))),
        })
    }
}

fn root_snapshot(data: &Data, request: SnapshotRequest, from_seq: i64) -> Snapshot {
    Snapshot {
        resource: ROOT_RESOURCE_URI.into(),
        from_seq,
        state: SnapshotState::Root(Box::new(RootState {
            agents: vec![AgentInfo {
                provider: "openai".into(),
                display_name: "OpenAI-compatible".into(),
                description: "A streaming OpenAI-compatible agent.".into(),
                models: vec![SessionModelInfo {
                    id: request.model_name.clone(),
                    provider: "openai".into(),
                    name: request.model_name,
                    max_context_window: None,
                    max_output_tokens: None,
                    max_prompt_tokens: None,
                    supports_vision: None,
                    policy_state: None,
                    config_schema: None,
                    meta: None,
                }],
                protected_resources: None,
                customizations: None,
                capabilities: Some(AgentCapabilities {
                    multiple_chats: Some(MultipleChatsCapability::default()),
                    multiple_working_directories: None,
                }),
            }],
            active_sessions: Some(i64::try_from(data.sessions.len()).unwrap_or(i64::MAX)),
            terminals: None,
            config: None,
            meta: None,
        })),
    }
}

fn session_snapshot(
    data: &Data,
    request: SnapshotRequest,
    from_seq: i64,
    session: &Session,
) -> Snapshot {
    let chats = session
        .chats
        .iter()
        .filter_map(|id| data.chats.get(id))
        .map(|chat| data.chat_summary(chat))
        .collect();
    let summary = data.session_summary(session);
    Snapshot {
        resource: request.channel,
        from_seq,
        state: SnapshotState::Session(Box::new(SessionState {
            provider: session.provider.clone(),
            title: summary.title,
            status: summary.status,
            activity: summary.activity,
            project: None,
            working_directories: Some(vec![session.workspace.clone()]),
            annotations: None,
            lifecycle: if session.ready {
                SessionLifecycle::Ready
            } else {
                SessionLifecycle::Creating
            },
            creation_error: None,
            server_tools: Some(request.server_tools),
            active_clients: Vec::new(),
            chats,
            default_chat: session.chats.first().cloned(),
            config: None,
            customizations: None,
            changesets: None,
            input_needed: None,
            meta: None,
        })),
    }
}

fn chat_state(chat: &Chat) -> ChatState {
    ChatState {
        resource: chat.uri.clone(),
        title: "New chat".into(),
        status: chat_status(chat),
        activity: chat_activity(chat),
        modified_at: chat.modified_at.clone(),
        origin: None,
        interactivity: None,
        working_directories: None,
        turns: chat.turns.iter().map(stored_turn).collect(),
        turns_next_cursor: None,
        active_turn: chat.active.as_ref().map(active_turn),
        steering_message: None,
        queued_messages: None,
        draft: None,
        meta: None,
    }
}

fn chat_status(chat: &Chat) -> u32 {
    if chat.active.as_ref().is_some_and(active_needs_input) {
        SessionStatus::InputNeeded.bits()
    } else if chat.active.is_some() {
        SessionStatus::InProgress.bits()
    } else if chat
        .turns
        .last()
        .is_some_and(|turn| turn.state == TurnState::Error)
    {
        SessionStatus::Error.bits()
    } else {
        SessionStatus::Idle.bits()
    }
}

fn chat_activity(chat: &Chat) -> Option<String> {
    chat.active.as_ref().map(|active| {
        if active_needs_input(active) {
            "Waiting for input".into()
        } else {
            "Waiting for model response".into()
        }
    })
}

fn active_needs_input(active: &Active) -> bool {
    active.response_parts.iter().any(|part| {
        matches!(
            part,
            ahp_types::state::ResponsePart::ToolCall(tool)
                if matches!(
                    tool.tool_call,
                    ahp_types::state::ToolCallState::PendingConfirmation(_)
                        | ahp_types::state::ToolCallState::PendingResultConfirmation(_)
                        | ahp_types::state::ToolCallState::AuthRequired(_)
                )
        )
    })
}

fn active_turn(active: &Active) -> ActiveTurn {
    ActiveTurn {
        id: active.id.clone(),
        started_at: active.started_at.clone(),
        message: active.message.clone(),
        response_parts: active.response_parts.clone(),
        usage: None,
    }
}

fn stored_turn(turn: &StoredTurn) -> Turn {
    Turn {
        id: turn.id.clone(),
        started_at: Some(turn.started_at.clone()),
        duration: Some(turn.duration),
        message: turn.message.clone(),
        response_parts: turn.response_parts.clone(),
        usage: None,
        state: turn.state,
        error: turn.error.clone(),
    }
}

#[cfg(test)]
mod tests;
