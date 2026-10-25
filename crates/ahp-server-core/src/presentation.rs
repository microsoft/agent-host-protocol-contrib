//! Canonical AHP server-tool state and action construction.

use ahp_types::actions::{
    ChatToolCallCompleteAction, ChatToolCallDeltaAction, ChatToolCallReadyAction,
    ChatToolCallStartAction, StateAction,
};
use ahp_types::state::{
    ConfirmationOption, ToolCallCancellationReason, ToolCallCancelledState, ToolCallCompletedState,
    ToolCallConfirmationReason, ToolCallPendingConfirmationState, ToolCallResponsePart,
    ToolCallRunningState, ToolCallState, ToolCallStreamingState, ToolInput,
};
use workspace_agent_tools::ToolExecution;

/// Stable display data for every lifecycle phase of one model tool call.
#[derive(Clone, Debug)]
pub(crate) struct ToolPresentation {
    pub(crate) id: String,
    pub(crate) name: String,
    display_name: String,
    intention: String,
    invocation_message: String,
    input: String,
    contributor: Option<ahp_types::state::ToolCallContributor>,
}

impl ToolPresentation {
    pub(crate) fn new(id: String, name: String, input: String) -> Result<Self, String> {
        if id.trim().is_empty() {
            return Err("tool call ID must not be empty".into());
        }
        let (display_name, intention) = match name.as_str() {
            "replace_string" => (
                "Replace text in file",
                "Replace one exact string in a workspace file",
            ),
            "run_shell" => (
                "Run shell command",
                "Run a command in the selected workspace",
            ),
            _ => ("Unknown server tool", "Execute an unsupported server tool"),
        };
        Ok(Self {
            id,
            name,
            display_name: display_name.into(),
            intention: intention.into(),
            invocation_message: intention.into(),
            input,
            contributor: None,
        })
    }

    pub(crate) fn for_turn(
        id: String,
        name: String,
        input: String,
        started: &crate::StartedTurn,
    ) -> Result<Self, String> {
        let mut presentation = Self::new(id, name, input)?;
        if let Some(tool) = started
            .client_tools
            .iter()
            .find(|tool| tool.definition.name == presentation.name)
        {
            presentation.display_name = tool
                .definition
                .title
                .clone()
                .unwrap_or_else(|| presentation.name.clone());
            presentation.intention = tool
                .definition
                .description
                .clone()
                .unwrap_or_else(|| format!("Run {}", presentation.name));
            presentation.invocation_message = presentation.intention.clone();
            presentation.contributor = Some(ahp_types::state::ToolCallContributor::Client(
                ahp_types::state::ToolCallClientContributor {
                    client_id: tool.client_id.clone(),
                },
            ));
        }
        Ok(presentation)
    }

    pub(crate) fn response_part(&self) -> ahp_types::state::ResponsePart {
        ahp_types::state::ResponsePart::ToolCall(Box::new(ToolCallResponsePart {
            tool_call: self.streaming_state(),
        }))
    }

    pub(crate) fn streaming_state(&self) -> ToolCallState {
        ToolCallState::Streaming(ToolCallStreamingState {
            tool_call_id: self.id.clone(),
            tool_name: self.name.clone(),
            display_name: self.display_name.clone(),
            intention: Some(self.intention.clone()),
            contributor: self.contributor.clone(),
            meta: None,
            partial_input: None,
            invocation_message: None,
        })
    }

    pub(crate) fn ready_state(
        &self,
        confirmed: ToolCallConfirmationReason,
        selected: Option<ConfirmationOption>,
    ) -> ToolCallState {
        ToolCallState::Running(ToolCallRunningState {
            tool_call_id: self.id.clone(),
            tool_name: self.name.clone(),
            display_name: self.display_name.clone(),
            intention: Some(self.intention.clone()),
            contributor: self.contributor.clone(),
            meta: None,
            invocation_message: self.invocation_message.clone().into(),
            tool_input: Some(ToolInput::Inline(self.input.clone())),
            confirmed,
            selected_option: selected,
            content: None,
        })
    }

    pub(crate) fn pending_state(&self, options: Vec<ConfirmationOption>) -> ToolCallState {
        ToolCallState::PendingConfirmation(ToolCallPendingConfirmationState {
            tool_call_id: self.id.clone(),
            tool_name: self.name.clone(),
            display_name: self.display_name.clone(),
            intention: Some(self.intention.clone()),
            contributor: self.contributor.clone(),
            meta: None,
            invocation_message: self.invocation_message.clone().into(),
            tool_input: Some(ToolInput::Inline(self.input.clone())),
            confirmation_title: Some("Shell command approval".into()),
            risk_assessment: None,
            edits: None,
            editable: Some(false),
            options: Some(options),
        })
    }

    pub(crate) fn completed_state(
        &self,
        result: &ToolExecution,
        confirmed: ToolCallConfirmationReason,
        selected: Option<ConfirmationOption>,
    ) -> ToolCallState {
        ToolCallState::Completed(ToolCallCompletedState {
            tool_call_id: self.id.clone(),
            tool_name: self.name.clone(),
            display_name: self.display_name.clone(),
            intention: Some(self.intention.clone()),
            contributor: self.contributor.clone(),
            meta: None,
            invocation_message: self.invocation_message.clone().into(),
            tool_input: Some(ToolInput::Inline(self.input.clone())),
            success: result.success,
            past_tense_message: result.past_tense_message.clone().into(),
            content: Some(result.content.clone()),
            structured_content: None,
            error: result.error.clone(),
            confirmed,
            selected_option: selected,
        })
    }

    pub(crate) fn denied_state(&self, selected: ConfirmationOption) -> ToolCallState {
        ToolCallState::Cancelled(ToolCallCancelledState {
            tool_call_id: self.id.clone(),
            tool_name: self.name.clone(),
            display_name: self.display_name.clone(),
            intention: Some(self.intention.clone()),
            contributor: self.contributor.clone(),
            meta: None,
            invocation_message: self.invocation_message.clone().into(),
            tool_input: Some(ToolInput::Inline(self.input.clone())),
            reason: ToolCallCancellationReason::Denied,
            reason_message: Some("Shell command was denied by the user.".into()),
            user_suggestion: None,
            selected_option: Some(selected),
        })
    }

    pub(crate) fn start_action(&self, turn_id: &str) -> StateAction {
        StateAction::ChatToolCallStart(ChatToolCallStartAction {
            turn_id: turn_id.into(),
            tool_call_id: self.id.clone(),
            meta: None,
            tool_name: self.name.clone(),
            display_name: self.display_name.clone(),
            intention: Some(self.intention.clone()),
            contributor: self.contributor.clone(),
        })
    }

    pub(crate) fn delta_action(&self, turn_id: &str, delta: String) -> StateAction {
        StateAction::ChatToolCallDelta(ChatToolCallDeltaAction {
            turn_id: turn_id.into(),
            tool_call_id: self.id.clone(),
            meta: None,
            content: Some(delta),
            invocation_message: None,
        })
    }

    pub(crate) fn ready_action(
        &self,
        turn_id: &str,
        confirmed: Option<ToolCallConfirmationReason>,
        options: Option<Vec<ConfirmationOption>>,
    ) -> StateAction {
        StateAction::ChatToolCallReady(ChatToolCallReadyAction {
            turn_id: turn_id.into(),
            tool_call_id: self.id.clone(),
            meta: None,
            contributor: self.contributor.clone(),
            intention: Some(self.intention.clone()),
            invocation_message: self.invocation_message.clone().into(),
            tool_input: Some(ToolInput::Inline(self.input.clone())),
            confirmation_title: options.as_ref().map(|_| "Shell command approval".into()),
            risk_assessment: None,
            edits: None,
            editable: Some(false),
            confirmed,
            options,
        })
    }

    pub(crate) fn complete_action(&self, turn_id: &str, result: &ToolExecution) -> StateAction {
        StateAction::ChatToolCallComplete(ChatToolCallCompleteAction {
            turn_id: turn_id.into(),
            tool_call_id: self.id.clone(),
            meta: None,
            result: result.to_call_result(),
            requires_result_confirmation: None,
        })
    }
}
