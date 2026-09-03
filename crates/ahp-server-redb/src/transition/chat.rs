//! Active-turn creation and text delta transitions.

use ahp_server_core::{CommittedTransition, StartedTurn, StoreConflict, StoreError, TurnStart};
use ahp_types::actions::{ChatDraftChangedAction, ChatResponsePartAction, StateAction};
use ahp_types::state::{MarkdownResponsePart, ResponsePart};

use crate::replay::{Draft, TransitionWork};
use crate::sessions::{load_session, write_chat};
use crate::types::{now_millis, now_rfc3339, response_part_id, ActiveRecord};
use crate::RedbHostStore;

use super::{owned_chat, summary_draft};

impl RedbHostStore {
    pub(crate) fn start_turn_transition(
        &self,
        request: TurnStart,
    ) -> Result<CommittedTransition<StartedTurn>, StoreError> {
        self.commit_transition(move |write| {
            let mut chat = owned_chat(write, &request.chat)?;
            if chat.active.is_some() {
                return Err(StoreConflict::ActiveTurn(request.chat).into());
            }
            let session = load_session(write, &chat.session_uri)?
                .ok_or_else(|| StoreConflict::UnknownChannel(chat.session_uri.clone()))?;
            let model_selection = request
                .action
                .message
                .model
                .clone()
                .or_else(|| chat.draft.as_ref().and_then(|draft| draft.model.clone()))
                .or(session.model_selection.clone())
                .or(request.legacy_default_selection)
                .ok_or_else(|| StoreConflict::MissingModelSelection(session.uri.clone()))?;
            let history = chat.turns.iter().map(crate::types::turn).collect();
            let model_cursor = chat.model_cursor.clone();
            let part = ResponsePart::Markdown(MarkdownResponsePart {
                id: response_part_id(&request.action.turn_id),
                content: String::new(),
            });
            let mut action = request.action;
            action.message.model = Some(model_selection.clone());
            chat.active = Some(ActiveRecord {
                id: action.turn_id.clone(),
                started_at: action.started_at.clone(),
                started_millis: now_millis(),
                message: action.message.clone(),
                response: String::new(),
                response_parts: vec![part.clone()],
            });
            chat.draft = None;
            chat.modified_at = now_rfc3339();
            write_chat(write, &chat)?;
            let value = StartedTurn {
                chat: request.chat.clone(),
                turn_id: action.turn_id.clone(),
                history,
                model_cursor,
                message: action.message.clone(),
                workspace: session
                    .working_directories
                    .into_iter()
                    .next()
                    .unwrap_or_default(),
                model_selection,
            };
            Ok(TransitionWork::new(
                value.clone(),
                vec![
                    Draft::action(
                        request.chat,
                        StateAction::ChatTurnStarted(action),
                        request.origin.clone(),
                    ),
                    Draft::action(
                        chat.uri.clone(),
                        StateAction::ChatDraftChanged(ChatDraftChangedAction { draft: None }),
                        request.origin,
                    ),
                    summary_draft(&chat),
                    Draft::action(
                        chat.uri,
                        StateAction::ChatResponsePart(ChatResponsePartAction {
                            turn_id: value.turn_id,
                            part,
                            meta: None,
                        }),
                        None,
                    ),
                ],
            ))
        })
    }
}
