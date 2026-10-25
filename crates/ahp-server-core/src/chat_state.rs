//! Shared AHP 1.0 pending-input transitions for memory and durable stores.

use ahp_types::actions::{
    ActionOrigin, ChatPendingMessageRemovedAction, ChatPendingMessageSetAction,
    ChatQueuedMessagesReorderedAction, ChatTruncatedAction, StateAction,
};
use ahp_types::state::{MessageKind, PendingMessage, PendingMessageKind};
use serde::{Deserialize, Serialize};

/// A client-authored chat state transition.
#[derive(Clone, Debug)]
pub struct ChangeChat {
    /// Target chat channel.
    pub chat: String,
    /// Validated mutation to persist and echo.
    pub mutation: ChatMutation,
    /// Originating client sequence, if supplied.
    pub origin: Option<ActionOrigin>,
}

/// Supported pending-input and history mutations.
#[derive(Clone, Debug)]
pub enum ChatMutation {
    /// Sets only this chat's independent read flag.
    IsRead(bool),
    /// Replaces steering input or upserts a queued message in place.
    PendingSet(ChatPendingMessageSetAction),
    /// Removes the matching identifier from the addressed kind of input.
    PendingRemoved(ChatPendingMessageRemovedAction),
    /// Reorders queued messages without discarding unmentioned entries.
    Reordered(ChatQueuedMessagesReorderedAction),
    /// Retains history through the named turn, or clears it when absent.
    Truncated(ChatTruncatedAction),
}

impl ChatMutation {
    /// The authoritative action corresponding to this mutation.
    #[must_use]
    pub fn action(&self) -> StateAction {
        match self {
            Self::IsRead(is_read) => {
                StateAction::ChatIsReadChanged(ahp_types::actions::ChatIsReadChangedAction {
                    is_read: *is_read,
                })
            }
            Self::PendingSet(action) => StateAction::ChatPendingMessageSet(action.clone()),
            Self::PendingRemoved(action) => StateAction::ChatPendingMessageRemoved(action.clone()),
            Self::Reordered(action) => StateAction::ChatQueuedMessagesReordered(action.clone()),
            Self::Truncated(action) => StateAction::ChatTruncated(action.clone()),
        }
    }
}

/// Queued input and the independent, single steering-message slot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PendingMessages {
    /// Queued messages, in host acceptance order unless explicitly reordered.
    #[serde(default)]
    pub entries: Vec<PendingMessage>,
    /// The most recently accepted steering message.
    #[serde(default)]
    pub steering: Option<PendingMessage>,
    /// Immutable transcripts accepted with each queued message (not wire state).
    #[serde(default)]
    pub queued_attachments: std::collections::BTreeMap<String, Vec<ahp_types::state::Turn>>,
    /// Immutable transcript accepted with the current steering message.
    #[serde(default)]
    pub steering_attachments: Vec<ahp_types::state::Turn>,
}

impl PendingMessages {
    /// Applies a pending-input mutation; truncation is handled by the store.
    pub fn apply(&mut self, mutation: &ChatMutation) -> Result<(), &'static str> {
        match mutation {
            ChatMutation::PendingSet(action) => {
                if action.id.trim().is_empty()
                    || !matches!(action.message.origin.kind, MessageKind::User)
                {
                    return Err("pending input requires a non-empty id and a user message");
                }
                let entry = PendingMessage {
                    id: action.id.clone(),
                    message: action.message.clone(),
                };
                if action.kind == PendingMessageKind::Steering {
                    self.steering_attachments.clear();
                    self.steering = Some(entry);
                } else if let Some(existing) =
                    self.entries.iter_mut().find(|entry| entry.id == action.id)
                {
                    self.queued_attachments.remove(&action.id);
                    *existing = entry;
                } else {
                    self.entries.push(entry);
                }
            }
            ChatMutation::PendingRemoved(action) => {
                if action.kind == PendingMessageKind::Steering {
                    if !self
                        .steering
                        .as_ref()
                        .is_some_and(|entry| entry.id == action.id)
                    {
                        return Err("no steering message with matching id");
                    }
                    self.steering = None;
                    self.steering_attachments.clear();
                } else {
                    let Some(index) = self.entries.iter().position(|entry| entry.id == action.id)
                    else {
                        return Err("no queued message with matching id");
                    };
                    self.entries.remove(index);
                    self.queued_attachments.remove(&action.id);
                }
            }
            ChatMutation::Reordered(action) => {
                let mut remaining = std::mem::take(&mut self.entries);
                for id in &action.order {
                    if let Some(index) = remaining.iter().position(|entry| &entry.id == id) {
                        self.entries.push(remaining.remove(index));
                    }
                }
                self.entries.extend(remaining);
            }
            ChatMutation::Truncated(_) | ChatMutation::IsRead(_) => {}
        }
        Ok(())
    }

    /// Applies `turnStarted.queuedMessageId` to both independent input slots.
    pub fn consume(&mut self, id: &str) -> Option<PendingMessage> {
        if self.steering.as_ref().is_some_and(|entry| entry.id == id) {
            self.steering = None;
            self.steering_attachments.clear();
        }
        self.queued_attachments.remove(id);
        let index = self.entries.iter().position(|entry| entry.id == id)?;
        Some(self.entries.remove(index))
    }
}

#[cfg(test)]
mod tests;
