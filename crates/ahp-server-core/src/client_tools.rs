//! Typed client-owned tool mutations shared by durable adapters.

use ahp_types::actions::{
    ActionOrigin, ChatToolCallCompleteAction, ChatToolCallContentChangedAction, StateAction,
};
use ahp_types::state::{
    SessionActiveClient, ToolCallCompletedState, ToolCallContributor, ToolCallResult,
    ToolCallState, ToolDefinition, ToolResultContent,
};

use crate::{StoreConflict, StoreError};

/// A registered client tool and its execution owner.
#[derive(Clone, Debug)]
pub struct ClientTool {
    /// Client responsible for completing invocations.
    pub client_id: String,
    /// Model-visible definition.
    pub definition: ToolDefinition,
}

/// Deduplicates registered tools, preferring the client that started the turn.
#[must_use]
pub fn client_tools(
    clients: &[SessionActiveClient],
    starting_client: Option<&str>,
) -> Vec<ClientTool> {
    let mut tools = Vec::new();
    let mut names = std::collections::HashSet::new();
    for client in clients
        .iter()
        .filter(|client| Some(client.client_id.as_str()) == starting_client)
        .chain(
            clients
                .iter()
                .filter(|client| Some(client.client_id.as_str()) != starting_client),
        )
    {
        for definition in &client.tools {
            if names.insert(definition.name.clone()) {
                tools.push(ClientTool {
                    client_id: client.client_id.clone(),
                    definition: definition.clone(),
                });
            }
        }
    }
    tools
}

/// A result or replacement live content supplied by a client executor.
#[derive(Clone, Debug)]
pub enum ClientToolMutation {
    /// Replaces, rather than appends, the running content.
    Content(Vec<ToolResultContent>),
    /// Completes a running invocation.
    Complete(ToolCallResult),
}

/// An atomic update to a client-owned running tool call.
#[derive(Clone, Debug)]
pub struct ChangeClientTool {
    /// Owning chat channel.
    pub chat: String,
    /// Active turn identifier.
    pub turn_id: String,
    /// Model-issued tool identifier.
    pub tool_call_id: String,
    /// Owner of the running invocation (also checked for host-generated failures).
    pub client_id: String,
    /// New content or terminal result.
    pub mutation: ClientToolMutation,
    /// Dispatching client, absent only for host-generated failure results.
    pub origin: Option<ActionOrigin>,
}

impl ChangeClientTool {
    /// Validates ownership and mutates only a currently running client tool.
    pub fn apply(&self, state: &mut ToolCallState) -> Result<(), StoreError> {
        let invalid = || StoreConflict::InvalidToolState {
            chat: self.chat.clone(),
            tool_call_id: self.tool_call_id.clone(),
        };
        let ToolCallState::Running(running) = state else {
            return Err(invalid().into());
        };
        if running.tool_call_id != self.tool_call_id
            || !matches!(&running.contributor, Some(ToolCallContributor::Client(owner)) if owner.client_id == self.client_id)
            || self
                .origin
                .as_ref()
                .is_some_and(|origin| origin.client_id != self.client_id)
        {
            return Err(invalid().into());
        }
        match &self.mutation {
            ClientToolMutation::Content(content) => running.content = Some(content.clone()),
            ClientToolMutation::Complete(result) => {
                *state = ToolCallState::Completed(ToolCallCompletedState {
                    tool_call_id: running.tool_call_id.clone(),
                    tool_name: running.tool_name.clone(),
                    display_name: running.display_name.clone(),
                    intention: running.intention.clone(),
                    contributor: running.contributor.clone(),
                    meta: running.meta.clone(),
                    invocation_message: running.invocation_message.clone(),
                    tool_input: running.tool_input.clone(),
                    confirmed: running.confirmed.clone(),
                    selected_option: running.selected_option.clone(),
                    success: result.success,
                    past_tense_message: result.past_tense_message.clone(),
                    content: result.content.clone(),
                    structured_content: result.structured_content.clone(),
                    error: result.error.clone(),
                });
            }
        }
        Ok(())
    }

    /// Action corresponding exactly to this mutation.
    #[must_use]
    pub fn action(&self) -> StateAction {
        match &self.mutation {
            ClientToolMutation::Content(content) => {
                StateAction::ChatToolCallContentChanged(ChatToolCallContentChangedAction {
                    turn_id: self.turn_id.clone(),
                    tool_call_id: self.tool_call_id.clone(),
                    meta: None,
                    content: content.clone(),
                })
            }
            ClientToolMutation::Complete(result) => {
                StateAction::ChatToolCallComplete(ChatToolCallCompleteAction {
                    turn_id: self.turn_id.clone(),
                    tool_call_id: self.tool_call_id.clone(),
                    meta: None,
                    result: result.clone(),
                    requires_result_confirmation: None,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::client_tools;
    use serde_json::json;

    #[test]
    fn tool_name_deduplication_prefers_the_starting_client() -> Result<(), serde_json::Error> {
        let clients: Vec<ahp_types::state::SessionActiveClient> = serde_json::from_value(json!([
            {"clientId":"first", "tools":[{"name":"shared"},{"name":"first-only"}]},
            {"clientId":"starter", "tools":[{"name":"shared"},{"name":"starter-only"}]}
        ]))?;
        let tools = client_tools(&clients, Some("starter"));
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].definition.name, "shared");
        assert_eq!(tools[0].client_id, "starter");
        assert_eq!(tools[2].definition.name, "first-only");
        Ok(())
    }
}
