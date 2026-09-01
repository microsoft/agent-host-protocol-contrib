//! Active-turn creation and text delta transitions.

use ahp_server_core::{CommittedTransition, StartedTurn, StoreConflict, StoreError, TurnStart};
use ahp_types::actions::{ChatDeltaAction, ChatResponsePartAction, StateAction};
use ahp_types::state::{MarkdownResponsePart, ResponsePart};

use crate::replay::{Draft, TransitionWork};
use crate::sessions::{load_session, write_chat};
use crate::types::{now_millis, now_rfc3339, response_part_id, ActiveRecord};
use crate::RedbHostStore;

use super::{active_mut, owned_chat, summary_draft};

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
                .or(session.model_selection.clone())
                .unwrap_or_else(|| request.legacy_default_selection.clone());
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
            chat.modified_at = now_rfc3339();
            write_chat(write, &chat)?;
            let value = StartedTurn {
                chat: request.chat.clone(),
                turn_id: action.turn_id.clone(),
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

    pub(crate) fn append_text_transition(
        &self,
        chat_uri: &str,
        turn_id: &str,
        chunk: &str,
    ) -> Result<CommittedTransition<()>, StoreError> {
        if chunk.is_empty() {
            return Err(StoreError::Backend("text deltas must not be empty".into()));
        }
        let (chat_uri, turn_id, chunk) =
            (chat_uri.to_owned(), turn_id.to_owned(), chunk.to_owned());
        self.commit_transition(move |write| {
            let mut chat = owned_chat(write, &chat_uri)?;
            let active = active_mut(&mut chat, &turn_id)?;
            let part_id = response_part_id(&turn_id);
            let markdown = active
                .response_parts
                .iter_mut()
                .find_map(|part| match part {
                    ResponsePart::Markdown(markdown) if markdown.id == part_id => Some(markdown),
                    _ => None,
                });
            let markdown = markdown.ok_or_else(|| {
                StoreError::Backend("active turn lacks its markdown response part".into())
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
