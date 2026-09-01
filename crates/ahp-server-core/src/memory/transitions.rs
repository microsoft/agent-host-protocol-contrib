//! Atomic active-turn and tool-call transitions.

use ahp_types::actions::{
    ChatDeltaAction, ChatResponsePartAction, SessionChatUpdatedAction, StateAction,
};
use ahp_types::state::{MarkdownResponsePart, ResponsePart, ToolCallState};

use super::state::{now, response_part_id};
use super::{Draft, MemoryHostStore};
use crate::store::{
    CommittedTransition, StartedTurn, StoreConflict, StoreError, ToolCallTransition, TurnStart,
};

pub(super) fn start_turn(
    store: &MemoryHostStore,
    request: TurnStart,
) -> Result<CommittedTransition<StartedTurn>, StoreError> {
    store.transact(move |data| {
        let session = data.session_for_chat(&request.chat)?;
        let workspace = session.workspace.clone();
        let model_selection = request
            .action
            .message
            .model
            .clone()
            .unwrap_or_else(|| session.model_selection.clone());
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
        chat.modified_at = now();
        let value = StartedTurn {
            chat: request.chat.clone(),
            turn_id: action.turn_id.clone(),
            message: action.message.clone(),
            workspace,
            model_selection,
        };
        let summary = data.chat_summary(data.chat(&request.chat)?);
        Ok((
            value.clone(),
            vec![
                Draft::action(
                    request.chat.clone(),
                    StateAction::ChatTurnStarted(action),
                    request.origin,
                ),
                summary_draft(data, &request.chat, summary),
                Draft::action(
                    request.chat,
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

pub(super) fn append_text(
    store: &MemoryHostStore,
    chat: &str,
    turn_id: &str,
    chunk: &str,
) -> Result<CommittedTransition<()>, StoreError> {
    let (chat, turn_id, chunk) = (chat.to_owned(), turn_id.to_owned(), chunk.to_owned());
    store.transact(move |data| {
        let active = data.active_mut(&chat, &turn_id)?;
        let part_id = response_part_id(&turn_id);
        let Some(ResponsePart::Markdown(markdown)) = active.response_parts.iter_mut().find(
            |part| matches!(part, ResponsePart::Markdown(markdown) if markdown.id == part_id),
        ) else {
            return Err(StoreError::Backend(
                "active turn lacks its markdown part".into(),
            ));
        };
        markdown.content.push_str(&chunk);
        data.chat_mut(&chat)?.modified_at = now();
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
        data.chat_mut(&chat)?.modified_at = now();
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
        data.chat_mut(&chat)?.modified_at = now();
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
        data.chat_mut(&transition.chat)?.modified_at = now();
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
                origin: None,
                interactivity: None,
                working_directories: None,
            },
        }),
        None,
    )
}
