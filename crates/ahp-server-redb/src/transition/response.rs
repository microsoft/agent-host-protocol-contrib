//! Crash-atomic response-part and markdown-delta updates.

use ahp_server_core::{CommittedTransition, StoreError};
use ahp_types::actions::{ChatDeltaAction, ChatResponsePartAction, StateAction};
use ahp_types::state::ResponsePart;

use crate::replay::{Draft, TransitionWork};
use crate::sessions::write_chat;
use crate::types::now_rfc3339;
use crate::RedbHostStore;

use super::{active_mut, owned_chat};

impl RedbHostStore {
    pub(crate) fn append_response_part_transition(
        &self,
        chat_uri: &str,
        turn_id: &str,
        part: ResponsePart,
    ) -> Result<CommittedTransition<()>, StoreError> {
        let (chat_uri, turn_id) = (chat_uri.to_owned(), turn_id.to_owned());
        self.commit_transition(move |write| {
            let mut chat = owned_chat(write, &chat_uri)?;
            active_mut(&mut chat, &turn_id)?
                .response_parts
                .push(part.clone());
            chat.modified_at = now_rfc3339();
            write_chat(write, &chat)?;
            Ok(TransitionWork::new(
                (),
                vec![Draft::action(
                    chat_uri,
                    StateAction::ChatResponsePart(ChatResponsePartAction {
                        turn_id,
                        part,
                        meta: None,
                    }),
                    None,
                )],
            ))
        })
    }

    pub(crate) fn append_text_transition(
        &self,
        chat_uri: &str,
        turn_id: &str,
        part_id: &str,
        chunk: &str,
    ) -> Result<CommittedTransition<()>, StoreError> {
        if chunk.is_empty() {
            return Err(StoreError::Backend("text deltas must not be empty".into()));
        }
        let (chat_uri, turn_id, part_id, chunk) = (
            chat_uri.to_owned(),
            turn_id.to_owned(),
            part_id.to_owned(),
            chunk.to_owned(),
        );
        self.commit_transition(move |write| {
            let mut chat = owned_chat(write, &chat_uri)?;
            let active = active_mut(&mut chat, &turn_id)?;
            let markdown = active
                .response_parts
                .iter_mut()
                .find_map(|part| match part {
                    ResponsePart::Markdown(markdown) if markdown.id == part_id => Some(markdown),
                    _ => None,
                });
            let markdown = markdown.ok_or_else(|| {
                StoreError::Backend("active turn lacks the requested markdown response part".into())
            })?;
            markdown.content.push_str(&chunk);
            active.response.push_str(&chunk);
            chat.modified_at = now_rfc3339();
            write_chat(write, &chat)?;
            Ok(TransitionWork::new(
                (),
                vec![Draft::action(
                    chat_uri,
                    StateAction::ChatDelta(ChatDeltaAction {
                        turn_id,
                        part_id,
                        content: chunk,
                        meta: None,
                    }),
                    None,
                )],
            ))
        })
    }
}
