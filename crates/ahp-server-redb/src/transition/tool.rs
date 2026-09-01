//! Server-tool lifecycle transitions.

use ahp_server_core::{CommittedTransition, StoreConflict, StoreError, ToolCallTransition};
use ahp_types::actions::StateAction;
use ahp_types::state::{ResponsePart, ToolCallState};

use crate::approvals::add_approval;
use crate::replay::{Draft, TransitionWork};
use crate::sessions::write_chat;
use crate::types::{now_rfc3339, part_tool_call_id, tool_call_id, valid_tool_transition};
use crate::RedbHostStore;

use super::{active_mut, owned_chat};

impl RedbHostStore {
    pub(crate) fn start_tool_transition(
        &self,
        chat_uri: &str,
        turn_id: &str,
        part: ResponsePart,
        action: StateAction,
    ) -> Result<CommittedTransition<()>, StoreError> {
        let tool_call_id = part_tool_call_id(&part)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| StoreError::Backend("tool start lacks a tool-call id".into()))?
            .to_owned();
        let (chat_uri, turn_id) = (chat_uri.to_owned(), turn_id.to_owned());
        self.commit_transition(move |write| {
            let mut chat = owned_chat(write, &chat_uri)?;
            let active = active_mut(&mut chat, &turn_id)?;
            if active
                .response_parts
                .iter()
                .filter_map(part_tool_call_id)
                .any(|existing| existing == tool_call_id)
            {
                return Err(StoreConflict::DuplicateToolCallId {
                    chat: chat_uri,
                    tool_call_id,
                }
                .into());
            }
            active.response_parts.push(part);
            chat.modified_at = now_rfc3339();
            write_chat(write, &chat)?;
            Ok(TransitionWork::new(
                (),
                vec![Draft::action(chat_uri, action, None)],
            ))
        })
    }

    pub(crate) fn append_tool_delta_transition(
        &self,
        chat_uri: &str,
        turn_id: &str,
        tool_id: &str,
        delta: &str,
        action: StateAction,
    ) -> Result<CommittedTransition<()>, StoreError> {
        if delta.is_empty() {
            return Err(StoreError::Backend("tool deltas must not be empty".into()));
        }
        let (chat_uri, turn_id, requested_tool_call_id, delta) = (
            chat_uri.to_owned(),
            turn_id.to_owned(),
            tool_id.to_owned(),
            delta.to_owned(),
        );
        self.commit_transition(move |write| {
            let mut chat = owned_chat(write, &chat_uri)?;
            let active = active_mut(&mut chat, &turn_id)?;
            let state = active
                .response_parts
                .iter_mut()
                .find_map(|part| match part {
                    ResponsePart::ToolCall(tool)
                        if tool_call_id(&tool.tool_call) == requested_tool_call_id =>
                    {
                        Some(&mut tool.tool_call)
                    }
                    _ => None,
                })
                .ok_or_else(|| StoreConflict::ToolNotFound {
                    chat: chat_uri.clone(),
                    tool_call_id: requested_tool_call_id.clone(),
                })?;
            let ToolCallState::Streaming(streaming) = state else {
                return Err(StoreConflict::InvalidToolState {
                    chat: chat_uri,
                    tool_call_id: requested_tool_call_id,
                }
                .into());
            };
            streaming
                .partial_input
                .get_or_insert_with(String::new)
                .push_str(&delta);
            chat.modified_at = now_rfc3339();
            write_chat(write, &chat)?;
            Ok(TransitionWork::new(
                (),
                vec![Draft::action(chat_uri, action, None)],
            ))
        })
    }

    pub(crate) fn transition_tool_transition(
        &self,
        transition: ToolCallTransition,
    ) -> Result<CommittedTransition<()>, StoreError> {
        if transition.actions.is_empty() {
            return Err(StoreError::Backend(
                "tool transition must contain an action envelope".into(),
            ));
        }
        self.commit_transition(move |write| {
            let mut chat = owned_chat(write, &transition.chat)?;
            let active = active_mut(&mut chat, &transition.turn_id)?;
            let state = active
                .response_parts
                .iter_mut()
                .find_map(|part| match part {
                    ResponsePart::ToolCall(tool)
                        if tool_call_id(&tool.tool_call) == transition.tool_call_id =>
                    {
                        Some(&mut tool.tool_call)
                    }
                    _ => None,
                })
                .ok_or_else(|| StoreConflict::ToolNotFound {
                    chat: transition.chat.clone(),
                    tool_call_id: transition.tool_call_id.clone(),
                })?;
            if !valid_tool_transition(state, &transition.next_state) {
                return Err(StoreConflict::InvalidToolState {
                    chat: transition.chat,
                    tool_call_id: transition.tool_call_id,
                }
                .into());
            }
            *state = transition.next_state;
            chat.modified_at = now_rfc3339();
            write_chat(write, &chat)?;
            if let Some(approval) = transition.allow_command.as_ref() {
                add_approval(write, approval)?;
            }
            let drafts = transition
                .actions
                .into_iter()
                .map(|action| Draft::action(chat.uri.clone(), action, transition.origin.clone()))
                .collect();
            Ok(TransitionWork::new((), drafts))
        })
    }
}
