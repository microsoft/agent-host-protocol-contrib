//! Active-turn creation and text delta transitions.

use ahp_server_core::{CommittedTransition, StartedTurn, StoreConflict, StoreError, TurnStart};
use ahp_types::actions::{ChatDraftChangedAction, ChatResponsePartAction, StateAction};
use ahp_types::state::{MarkdownResponsePart, ResponsePart};

use crate::replay::{Draft, TransitionWork};
use crate::sessions::{load_session, write_chat};
use crate::types::{now_millis, response_part_id, ActiveRecord};
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
            let pinned = request.action.queued_message_id.as_ref().and_then(|id| {
                chat.pending.entries.iter().find(|entry| {
                    entry.id == *id
                        && entry.message.attachments == request.action.message.attachments
                })?;
                chat.pending.queued_attachments.get(id).cloned()
            });
            let attachment_context = match pinned {
                Some(context) => context,
                None => ahp_server_core::materialize_attachments(&request.action.message, |uri| {
                    let source = owned_chat(write, uri)?;
                    Ok(source.turns.iter().map(crate::types::turn).collect())
                })?,
            };
            chat.materialized_attachments
                .retain(|id, _| chat.turns.iter().any(|turn| turn.id == *id));
            chat.materialized_attachments
                .insert(request.action.turn_id.clone(), attachment_context);
            let client_tools = ahp_server_core::client_tools(
                &session.active_clients,
                request
                    .origin
                    .as_ref()
                    .map(|origin| origin.client_id.as_str()),
            );
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
            chat.modified_at = action.started_at.clone();
            chat.status_reset = false;
            chat.status_flags &= !ahp_types::state::SessionStatus::IsRead.bits();
            let consumed = action
                .queued_message_id
                .as_deref()
                .and_then(|id| chat.pending.consume(id));
            write_chat(write, &chat)?;
            let value = StartedTurn {
                chat: request.chat.clone(),
                turn_id: action.turn_id.clone(),
                history,
                model_cursor,
                materialized_attachments: chat.materialized_attachments.clone(),
                session: session.uri.clone(),
                client_tools,
                message: action.message.clone(),
                workspace: session
                    .working_directories
                    .into_iter()
                    .next()
                    .unwrap_or_default(),
                model_selection,
            };
            let mut drafts = vec![
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
                        turn_id: value.turn_id.clone(),
                        part,
                        meta: None,
                    }),
                    None,
                ),
            ];
            if let Some(entry) = consumed {
                drafts.insert(
                    0,
                    Draft::action(
                        value.chat.clone(),
                        StateAction::ChatPendingMessageRemoved(
                            ahp_types::actions::ChatPendingMessageRemovedAction {
                                kind: ahp_types::state::PendingMessageKind::Queued,
                                id: entry.id,
                            },
                        ),
                        None,
                    ),
                );
            }
            Ok(TransitionWork::new(value, drafts))
        })
    }
}
