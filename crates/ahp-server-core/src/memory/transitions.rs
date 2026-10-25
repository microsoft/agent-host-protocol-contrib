//! Atomic active-turn and tool-call transitions.

use ahp_types::actions::{
    ChatDeltaAction, ChatDraftChangedAction, ChatResponsePartAction, SessionChatUpdatedAction,
    StateAction,
};
use ahp_types::state::{MarkdownResponsePart, MessageKind, ResponsePart, ToolCallState};

use super::state::response_part_id;
use super::{Draft, MemoryHostStore};
use crate::store::{
    ChangeDraft, CommittedTransition, DraftTurnPrecondition, StartedTurn, StoreConflict,
    StoreError, ToolCallTransition, TurnStart,
};

pub(super) fn start_turn(
    store: &MemoryHostStore,
    request: TurnStart,
) -> Result<CommittedTransition<StartedTurn>, StoreError> {
    store.transact(move |data| {
        let pinned = request.action.queued_message_id.as_ref().and_then(|id| {
            let pending = &data.chat(&request.chat).ok()?.pending;
            pending.entries.iter().find(|entry| {
                entry.id == *id && entry.message.attachments == request.action.message.attachments
            })?;
            pending.queued_attachments.get(id).cloned()
        });
        let attachment_context = match pinned {
            Some(context) => context,
            None => crate::materialize_attachments(&request.action.message, |uri| {
                Ok(data
                    .chat(uri)?
                    .turns
                    .iter()
                    .map(super::state::StoredTurn::snapshot)
                    .collect())
            })?,
        };
        let existing_chat = data.chat(&request.chat)?;
        let mut materialized_attachments = existing_chat.materialized_attachments.clone();
        materialized_attachments
            .retain(|id, _| existing_chat.turns.iter().any(|turn| turn.id == *id));
        materialized_attachments.insert(request.action.turn_id.clone(), attachment_context);
        let draft_selection = existing_chat
            .draft
            .as_ref()
            .and_then(|draft| draft.model.clone());
        let history = existing_chat
            .turns
            .iter()
            .map(super::state::StoredTurn::snapshot)
            .collect();
        let model_cursor = existing_chat.model_cursor.clone();
        let session = data.session_for_chat(&request.chat)?;
        let workspace = session.workspace.clone();
        let session_uri = session.uri.clone();
        let client_tools = crate::client_tools(
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
            .or(draft_selection)
            .or(session.model_selection.clone())
            .or(request.legacy_default_selection)
            .ok_or_else(|| StoreConflict::MissingModelSelection(session.uri.clone()))?;
        let chat = data.chat_mut(&request.chat)?;
        if chat.active.is_some() {
            return Err(StoreConflict::ActiveTurn(request.chat).into());
        }
        let part = ResponsePart::Markdown(MarkdownResponsePart {
            id: response_part_id(&request.action.turn_id),
            content: String::new(),
        });
        let mut action = request.action;
        action.message.model = Some(model_selection.clone());
        chat.active = Some(super::state::Active::new(&action, part.clone()));
        chat.materialized_attachments = materialized_attachments.clone();
        chat.draft = None;
        chat.modified_at = action.started_at.clone();
        chat.status_reset = false;
        chat.status_flags &= !ahp_types::state::SessionStatus::IsRead.bits();
        let consumed = action
            .queued_message_id
            .as_deref()
            .and_then(|id| chat.pending.consume(id));
        let value = StartedTurn {
            chat: request.chat.clone(),
            turn_id: action.turn_id.clone(),
            history,
            model_cursor,
            materialized_attachments,
            session: session_uri,
            client_tools,
            message: action.message.clone(),
            workspace,
            model_selection,
        };
        let summary = data.chat_summary(data.chat(&request.chat)?);
        let mut drafts = vec![
            Draft::action(
                request.chat.clone(),
                StateAction::ChatTurnStarted(action),
                request.origin.clone(),
            ),
            Draft::action(
                request.chat.clone(),
                StateAction::ChatDraftChanged(ChatDraftChangedAction { draft: None }),
                request.origin,
            ),
            summary_draft(data, &request.chat, summary),
            Draft::action(
                request.chat,
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
        Ok((value, drafts))
    })
}

pub(super) fn change_draft(
    store: &MemoryHostStore,
    request: ChangeDraft,
) -> Result<CommittedTransition<()>, StoreError> {
    store.transact(move |data| {
        if request
            .draft
            .as_ref()
            .is_some_and(|draft| !matches!(draft.origin.kind, MessageKind::User))
        {
            return Err(StoreConflict::InvalidDraft(request.chat).into());
        }
        let chat = data.chat_mut(&request.chat)?;
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
        Ok((
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

pub(super) fn append_response_part(
    store: &MemoryHostStore,
    chat: &str,
    turn_id: &str,
    part: ResponsePart,
) -> Result<CommittedTransition<()>, StoreError> {
    let (chat, turn_id) = (chat.to_owned(), turn_id.to_owned());
    store.transact(move |data| {
        data.active_mut(&chat, &turn_id)?
            .response_parts
            .push(part.clone());
        Ok((
            (),
            vec![Draft::action(
                chat,
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

pub(super) fn append_text(
    store: &MemoryHostStore,
    chat: &str,
    turn_id: &str,
    part_id: &str,
    chunk: &str,
) -> Result<CommittedTransition<()>, StoreError> {
    let (chat, turn_id, part_id, chunk) = (
        chat.to_owned(),
        turn_id.to_owned(),
        part_id.to_owned(),
        chunk.to_owned(),
    );
    store.transact(move |data| {
        let active = data.active_mut(&chat, &turn_id)?;
        let Some(ResponsePart::Markdown(markdown)) = active.response_parts.iter_mut().find(
            |part| matches!(part, ResponsePart::Markdown(markdown) if markdown.id == part_id),
        ) else {
            return Err(StoreError::Backend(
                "active turn lacks its markdown part".into(),
            ));
        };
        markdown.content.push_str(&chunk);
        Ok((
            (),
            vec![Draft::action(
                chat,
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

pub(super) fn start_tool(
    store: &MemoryHostStore,
    chat: &str,
    turn_id: &str,
    part: ResponsePart,
    action: StateAction,
) -> Result<CommittedTransition<()>, StoreError> {
    let (chat, turn_id) = (chat.to_owned(), turn_id.to_owned());
    store.transact(move |data| {
        let id = super::state::part_tool_id(&part)
            .ok_or_else(|| StoreError::Backend("tool start lacks a tool call".into()))?
            .to_owned();
        let active = data.active_mut(&chat, &turn_id)?;
        if active
            .response_parts
            .iter()
            .filter_map(super::state::part_tool_id)
            .any(|existing| existing == id)
        {
            return Err(StoreConflict::DuplicateToolCallId {
                chat,
                tool_call_id: id,
            }
            .into());
        }
        active.response_parts.push(part);
        Ok(((), vec![Draft::action(chat, action, None)]))
    })
}

pub(super) fn append_tool_delta(
    store: &MemoryHostStore,
    chat: &str,
    turn_id: &str,
    tool_call_id: &str,
    delta: &str,
    action: StateAction,
) -> Result<CommittedTransition<()>, StoreError> {
    let (chat, turn_id, tool_call_id, delta) = (
        chat.to_owned(),
        turn_id.to_owned(),
        tool_call_id.to_owned(),
        delta.to_owned(),
    );
    store.transact(move |data| {
        let state = data.tool_state_mut(&chat, &turn_id, &tool_call_id)?;
        let ToolCallState::Streaming(streaming) = state else {
            return Err(StoreConflict::InvalidToolState { chat, tool_call_id }.into());
        };
        streaming
            .partial_input
            .get_or_insert_with(String::new)
            .push_str(&delta);
        Ok(((), vec![Draft::action(chat, action, None)]))
    })
}

pub(super) fn transition_tool(
    store: &MemoryHostStore,
    transition: ToolCallTransition,
) -> Result<CommittedTransition<()>, StoreError> {
    store.transact(move |data| {
        let current = data.tool_state_mut(
            &transition.chat,
            &transition.turn_id,
            &transition.tool_call_id,
        )?;
        if !super::state::valid_tool_edge(current, &transition.next_state) {
            return Err(StoreConflict::InvalidToolState {
                chat: transition.chat,
                tool_call_id: transition.tool_call_id,
            }
            .into());
        }
        *current = transition.next_state;
        if let Some(command) = transition.allow_command {
            data.approvals.insert(command);
        }
        let drafts = transition
            .actions
            .into_iter()
            .map(|action| Draft::action(transition.chat.clone(), action, transition.origin.clone()))
            .collect();
        Ok(((), drafts))
    })
}

pub(super) fn summary_draft(
    data: &super::state::Data,
    chat_uri: &str,
    summary: ahp_types::state::ChatSummary,
) -> Draft {
    let session = data
        .chats
        .get(chat_uri)
        .map(|chat| chat.session.clone())
        .unwrap_or_default();
    Draft::action(
        session,
        StateAction::SessionChatUpdated(SessionChatUpdatedAction {
            chat: chat_uri.into(),
            changes: ahp_types::actions::PartialChatSummary {
                resource: None,
                title: None,
                status: Some(summary.status),
                activity: summary.activity,
                modified_at: Some(summary.modified_at),
                changes: None,
                origin: None,
                movable: None,
                interactivity: None,
                working_directories: None,
            },
        }),
        None,
    )
}
