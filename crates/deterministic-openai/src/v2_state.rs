//! Tool-result progression state machine for deterministic fixture v2 and v4.

use super::contract::{code, ContractError};
use super::request::{FixtureRequest, ToolResult};
use super::v2::{V2Action, V2FixtureSpec};

impl V2FixtureSpec {
    pub(crate) fn next_action(&self, request: &FixtureRequest) -> Result<V2Action, ContractError> {
        let completed = completed_tool_results(request, self.calls.len(), "v2")?;
        match self.calls.get(completed.len()) {
            Some(call) => Ok(V2Action::ToolCall {
                id: tool_call_id(completed.len()),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            }),
            None => Ok(V2Action::FinalText(self.response.clone())),
        }
    }
}

pub(super) fn tool_call_id(index: usize) -> String {
    format!("ahp-test-tool-{}", index + 1)
}

pub(super) fn completed_tool_results<'a>(
    request: &'a FixtureRequest,
    call_count: usize,
    version: &str,
) -> Result<Vec<&'a ToolResult>, ContractError> {
    let completed = request.tool_results_after_last_user().collect::<Vec<_>>();
    for (index, result) in completed.iter().enumerate() {
        if index >= call_count {
            return Err(ContractError::new(
                code::INVALID_REQUEST,
                format!(
                    "AHP-TEST {version} received a tool result after the configured tool calls"
                ),
            ));
        }
        let expected_id = tool_call_id(index);
        if result.tool_call_id != expected_id {
            return Err(ContractError::new(
                code::INVALID_REQUEST,
                format!(
                    "AHP-TEST {version} expected tool result id {expected_id:?}, got {:?}",
                    result.tool_call_id
                ),
            ));
        }
    }
    Ok(completed)
}
