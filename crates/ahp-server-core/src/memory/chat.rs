//! Atomic pending-input and truncation transitions.

use super::transitions::summary_draft;
use super::{Draft, MemoryHostStore};
use crate::{ChangeChat, ChatMutation, CommittedTransition, StoreConflict, StoreError};

pub(super) fn change_chat(
    store: &MemoryHostStore,
    request: ChangeChat,
) -> Result<CommittedTransition<()>, StoreError> {
    store.transact(move |data| {
        let context = if let ChatMutation::PendingSet(action) = &request.mutation {
            Some(crate::materialize_attachments(&action.message, |uri| {
                Ok(data
                    .chat(uri)?
                    .turns
                    .iter()
                    .map(super::state::StoredTurn::snapshot)
                    .collect())
            })?)
        } else {
            None
        };
        let chat = data.chat_mut(&request.chat)?;
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
        if let (ChatMutation::PendingSet(action), Some(context)) = (&request.mutation, context) {
            if action.kind == ahp_types::state::PendingMessageKind::Steering {
                chat.pending.steering_attachments = context;
            } else {
                chat.pending
                    .queued_attachments
                    .insert(action.id.clone(), context);
            }
        }
        let mut drafts = vec![Draft::action(
            request.chat.clone(),
            request.mutation.action(),
            request.origin,
        )];
        if matches!(
            request.mutation,
            ChatMutation::Truncated(_) | ChatMutation::IsRead(_)
        ) {
            let summary = data.chat_summary(data.chat(&request.chat)?);
            drafts.push(summary_draft(data, &request.chat, summary));
        }
        Ok(((), drafts))
    })
}
