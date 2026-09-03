//! Crash-atomic chat draft updates.

use ahp_server_core::{
    ChangeDraft, CommittedTransition, DraftTurnPrecondition, StoreConflict, StoreError,
};
use ahp_types::actions::{ChatDraftChangedAction, StateAction};
use ahp_types::state::MessageKind;

use crate::replay::{Draft, TransitionWork};
use crate::sessions::write_chat;
use crate::types::now_rfc3339;
use crate::RedbHostStore;

use super::owned_chat;

impl RedbHostStore {
    pub(crate) fn change_draft_transition(
        &self,
        request: ChangeDraft,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.commit_transition(move |write| {
            if request
                .draft
                .as_ref()
                .is_some_and(|draft| !matches!(draft.origin.kind, MessageKind::User))
            {
                return Err(StoreConflict::InvalidDraft(request.chat).into());
            }
            let mut chat = owned_chat(write, &request.chat)?;
            if let DraftTurnPrecondition::Latest(expected) = &request.turn_precondition {
                let actual = chat
                    .active
                    .as_ref()
                    .map(|turn| turn.id.clone())
                    .or_else(|| chat.turns.last().map(|turn| turn.id.clone()));
                if expected != &actual {
                    return Err(StoreConflict::StaleDraft {
                        chat: request.chat,
                        expected: expected.clone(),
                        actual,
                    }
                    .into());
                }
            }
            chat.draft = request.draft.clone();
            chat.modified_at = now_rfc3339();
            write_chat(write, &chat)?;
            Ok(TransitionWork::new(
                (),
                vec![Draft::action(
                    request.chat,
                    StateAction::ChatDraftChanged(ChatDraftChangedAction {
                        draft: request.draft,
                    }),
                    request.origin,
                )],
            ))
        })
    }
}
