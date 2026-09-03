//! Private records and primitive mutations for the in-memory state adapter.

mod projection;
mod terminal;

use std::collections::{BTreeMap, HashSet};

use ahp_types::actions::ActionEnvelope;
use ahp_types::common::Uri;
use ahp_types::state::{
    ErrorInfo, ErrorResponsePart, Message, ResponsePart, ToolCallState, Turn, TurnState,
};
use chrono::{SecondsFormat, Utc};
use workspace_agent_tools::ExactCommandApproval;

use super::{Draft, REPLAY_LIMIT};
use crate::store::{StoreConflict, StoreError};
use crate::ModelCursor;

#[derive(Clone)]
pub(super) struct Data {
    pub(super) server_seq: u64,
    pub(super) first_sequence: i64,
    pub(super) replay: Vec<ActionEnvelope>,
    pub(super) sessions: BTreeMap<Uri, Session>,
    pub(super) chats: BTreeMap<Uri, Chat>,
    pub(super) approvals: HashSet<ExactCommandApproval>,
}

impl Default for Data {
    fn default() -> Self {
        Self {
            server_seq: 0,
            first_sequence: 1,
            replay: Vec::new(),
            sessions: BTreeMap::new(),
            chats: BTreeMap::new(),
            approvals: HashSet::new(),
        }
    }
}

#[derive(Clone)]
pub(super) struct Session {
    pub(super) uri: Uri,
    pub(super) provider: String,
    pub(super) workspace: Uri,
    pub(super) created_at: String,
    pub(super) ready: bool,
    pub(super) model_selection: Option<ahp_types::state::ModelSelection>,
    pub(super) chats: Vec<Uri>,
}

impl Session {
    pub(super) fn new(
        uri: Uri,
        provider: String,
        workspace: Uri,
        created_at: String,
        model_selection: Option<ahp_types::state::ModelSelection>,
    ) -> Self {
        Self {
            uri,
            provider,
            workspace,
            created_at,
            ready: false,
            model_selection,
            chats: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub(super) struct Chat {
    pub(super) uri: Uri,
    pub(super) session: Uri,
    pub(super) modified_at: String,
    pub(super) turns: Vec<StoredTurn>,
    pub(super) active: Option<Active>,
    pub(super) draft: Option<Message>,
    pub(super) model_cursor: Option<ModelCursor>,
}

impl Chat {
    pub(super) fn new(uri: Uri, session: Uri, timestamp: String) -> Self {
        Self {
            uri,
            session,
            modified_at: timestamp,
            turns: Vec::new(),
            active: None,
            draft: None,
            model_cursor: None,
        }
    }
}

#[derive(Clone)]
pub(super) struct Active {
    pub(super) id: String,
    pub(super) started_at: String,
    pub(super) started_millis: i64,
    pub(super) message: Message,
    pub(super) response_parts: Vec<ResponsePart>,
}

impl Active {
    pub(super) fn new(
        action: &ahp_types::actions::ChatTurnStartedAction,
        part: ResponsePart,
    ) -> Self {
        Self {
            id: action.turn_id.clone(),
            started_at: action.started_at.clone(),
            started_millis: Utc::now().timestamp_millis(),
            message: action.message.clone(),
            response_parts: vec![part],
        }
    }
}

#[derive(Clone)]
pub(super) struct StoredTurn {
    pub(super) id: String,
    pub(super) started_at: String,
    pub(super) duration: i64,
    pub(super) message: Message,
    pub(super) response_parts: Vec<ResponsePart>,
    pub(super) state: TurnState,
    pub(super) error: Option<ErrorInfo>,
}

impl StoredTurn {
    pub(super) fn snapshot(&self) -> Turn {
        let mut response_parts = self.response_parts.clone();
        if !response_parts
            .iter()
            .any(|part| matches!(part, ResponsePart::Error(_)))
        {
            if let Some(error) = &self.error {
                response_parts.push(ResponsePart::Error(ErrorResponsePart {
                    error: error.clone(),
                    resumable: None,
                }));
            }
        }
        Turn {
            id: self.id.clone(),
            started_at: Some(self.started_at.clone()),
            duration: Some(self.duration),
            message: self.message.clone(),
            response_parts,
            usage: None,
            state: self.state,
        }
    }
}

impl Data {
    pub(super) fn commit(&mut self, drafts: Vec<Draft>) -> Result<Vec<ActionEnvelope>, StoreError> {
        let mut envelopes = Vec::with_capacity(drafts.len());
        for draft in drafts {
            self.server_seq = self
                .server_seq
                .checked_add(1)
                .ok_or_else(|| StoreError::Backend("server sequence overflow".into()))?;
            envelopes.push(ActionEnvelope {
                channel: draft.channel,
                action: draft.action,
                server_seq: self.server_seq,
                origin: draft.origin,
                rejection_reason: draft.rejection_reason,
            });
        }
        self.replay.extend(envelopes.iter().cloned());
        let excess = self.replay.len().saturating_sub(REPLAY_LIMIT);
        if excess > 0 {
            self.replay.drain(..excess);
            self.first_sequence = self
                .replay
                .first()
                .and_then(|item| i64::try_from(item.server_seq).ok())
                .unwrap_or(i64::MAX);
        }
        Ok(envelopes)
    }

    pub(super) fn session(&self, uri: &str) -> Result<&Session, StoreError> {
        self.sessions
            .get(uri)
            .ok_or_else(|| StoreConflict::UnknownChannel(uri.into()).into())
    }

    pub(super) fn session_mut(&mut self, uri: &str) -> Result<&mut Session, StoreError> {
        self.sessions
            .get_mut(uri)
            .ok_or_else(|| StoreConflict::UnknownChannel(uri.into()).into())
    }

    pub(super) fn chat(&self, uri: &str) -> Result<&Chat, StoreError> {
        self.chats
            .get(uri)
            .ok_or_else(|| StoreConflict::UnknownChannel(uri.into()).into())
    }

    pub(super) fn chat_mut(&mut self, uri: &str) -> Result<&mut Chat, StoreError> {
        self.chats
            .get_mut(uri)
            .ok_or_else(|| StoreConflict::UnknownChannel(uri.into()).into())
    }

    pub(super) fn session_for_chat(&self, uri: &str) -> Result<&Session, StoreError> {
        self.session(&self.chat(uri)?.session)
    }

    pub(super) fn active_mut(&mut self, chat: &str, turn: &str) -> Result<&mut Active, StoreError> {
        let active = self
            .chat_mut(chat)?
            .active
            .as_mut()
            .ok_or_else(|| StoreConflict::NoActiveTurn(chat.into()))?;
        if active.id != turn {
            return Err(StoreConflict::TurnMismatch {
                chat: chat.into(),
                turn: turn.into(),
            }
            .into());
        }
        Ok(active)
    }

    pub(super) fn tool_state_mut(
        &mut self,
        chat: &str,
        turn: &str,
        id: &str,
    ) -> Result<&mut ToolCallState, StoreError> {
        let active = self.active_mut(chat, turn)?;
        active
            .response_parts
            .iter_mut()
            .find_map(|part| match part {
                ResponsePart::ToolCall(tool) if tool_state_id(&tool.tool_call) == id => {
                    Some(&mut tool.tool_call)
                }
                _ => None,
            })
            .ok_or_else(|| {
                StoreConflict::ToolNotFound {
                    chat: chat.into(),
                    tool_call_id: id.into(),
                }
                .into()
            })
    }
}

pub(super) fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(super) fn response_part_id(turn: &str) -> String {
    format!("response-{turn}")
}

pub(super) fn part_tool_id(part: &ResponsePart) -> Option<&str> {
    if let ResponsePart::ToolCall(call) = part {
        Some(tool_state_id(&call.tool_call))
    } else {
        None
    }
}

pub(super) fn tool_state_id(state: &ToolCallState) -> &str {
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

pub(super) fn valid_tool_edge(current: &ToolCallState, next: &ToolCallState) -> bool {
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
