//! Crash-atomic pending-input and truncation mutations.

use ahp_server_core::{ChangeChat, ChatMutation, CommittedTransition, StoreConflict, StoreError};

use super::{owned_chat, summary_draft};
use crate::replay::{Draft, TransitionWork};
use crate::sessions::write_chat;
use crate::RedbHostStore;

impl RedbHostStore {
    pub(crate) fn change_chat_transition(
        &self,
        request: ChangeChat,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.commit_transition(move |write| {
            let mut chat = owned_chat(write, &request.chat)?;
            let context = if let ChatMutation::PendingSet(action) = &request.mutation {
                Some(ahp_server_core::materialize_attachments(
                    &action.message,
                    |uri| {
                        Ok(owned_chat(write, uri)?
                            .turns
                            .iter()
                            .map(crate::types::turn)
                            .collect())
                    },
                )?)
            } else {
                None
            };
            if let ChatMutation::IsRead(is_read) = &request.mutation {
                let flag = ahp_types::state::SessionStatus::IsRead.bits();
                chat.status_flags = if *is_read {
                    chat.status_flags | flag
                } else {
                    chat.status_flags & !flag
                };
            } else if let ChatMutation::Truncated(action) = &request.mutation {
                let count = match &action.turn_id {
                    Some(id) => chat
                        .turns
                        .iter()
                        .position(|turn| &turn.id == id)
                        .map(|index| index + 1)
                        .ok_or_else(|| StoreConflict::TurnMismatch {
                            chat: request.chat.clone(),
                            turn: id.clone(),
                        })?,
                    None => 0,
                };
                chat.turns.truncate(count);
                chat.materialized_attachments
                    .retain(|id, _| chat.turns.iter().any(|turn| turn.id == *id));
                chat.active = None;
                chat.model_cursor = None;
                chat.status_reset = true;
            } else {
                chat.pending
                    .apply(&request.mutation)
                    .map_err(|message| StoreError::Backend(message.into()))?;
            }
            if let (ChatMutation::PendingSet(action), Some(context)) = (&request.mutation, context)
            {
                if action.kind == ahp_types::state::PendingMessageKind::Steering {
                    chat.pending.steering_attachments = context;
                } else {
                    chat.pending
                        .queued_attachments
                        .insert(action.id.clone(), context);
                }
            }
            write_chat(write, &chat)?;
            let mut drafts = vec![Draft::action(
                request.chat,
                request.mutation.action(),
                request.origin,
            )];
            if matches!(
                request.mutation,
                ChatMutation::Truncated(_) | ChatMutation::IsRead(_)
            ) {
                drafts.push(summary_draft(&chat));
            }
            Ok(TransitionWork::new((), drafts))
        })
    }
}
