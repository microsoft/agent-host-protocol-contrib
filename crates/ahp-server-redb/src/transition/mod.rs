//! Shared validation and session-summary drafting for state transitions.

mod chat;
mod terminal;
mod tool;

use ahp_server_core::{StoreConflict, StoreError};
use ahp_types::actions::{PartialChatSummary, SessionChatUpdatedAction, StateAction};

use crate::replay::Draft;
use crate::sessions::{load_chat, session_owns_chat};
use crate::types::{chat_summary, ActiveRecord, PersistedChat};

pub(super) fn owned_chat(
    write: &redb::WriteTransaction,
    chat_uri: &str,
) -> Result<PersistedChat, StoreError> {
    let chat = load_chat(write, chat_uri)?
        .ok_or_else(|| StoreConflict::UnknownChannel(chat_uri.into()))?;
    if !session_owns_chat(write, &chat)? {
        return Err(StoreConflict::UnknownChannel(chat_uri.into()).into());
    }
    Ok(chat)
}

pub(super) fn active_mut<'a>(
    chat: &'a mut PersistedChat,
    turn_id: &str,
) -> Result<&'a mut ActiveRecord, StoreError> {
    let active = chat
        .active
        .as_mut()
        .ok_or_else(|| StoreConflict::NoActiveTurn(chat.uri.clone()))?;
    if active.id != turn_id {
        return Err(StoreConflict::TurnMismatch {
            chat: chat.uri.clone(),
            turn: turn_id.into(),
        }
        .into());
    }
    Ok(active)
}

pub(super) fn summary_draft(chat: &PersistedChat) -> Draft {
    let summary = chat_summary(chat);
    Draft::action(
        chat.session_uri.clone(),
        StateAction::SessionChatUpdated(SessionChatUpdatedAction {
            chat: chat.uri.clone(),
            changes: PartialChatSummary {
                resource: None,
                title: None,
                status: Some(summary.status),
                activity: summary.activity,
                modified_at: Some(summary.modified_at),
                origin: None,
                interactivity: None,
                working_directories: None,
            },
        }),
        None,
    )
}
