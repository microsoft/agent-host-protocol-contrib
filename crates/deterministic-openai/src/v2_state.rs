//! Tool-result progression state machine for deterministic fixture v2.

use super::contract::{code, ContractError};
use super::request::FixtureRequest;
use super::v2::{V2Action, V2FixtureSpec};

impl V2FixtureSpec {
    pub(crate) fn next_action(&self, request: &FixtureRequest) -> Result<V2Action, ContractError> {
        let completed = request.tool_results_after_last_user().collect::<Vec<_>>();
        for (index, result) in completed.iter().enumerate() {
            if index >= self.calls.len() {
                return Err(ContractError::new(
                    code::INVALID_REQUEST,
                    "AHP-TEST v2 received a tool result after the configured tool calls",
                ));
            }
            let expected_id = tool_call_id(index);
            if result.tool_call_id != expected_id {
                return Err(ContractError::new(
                    code::INVALID_REQUEST,
                    format!(
                        "AHP-TEST v2 expected tool result id {expected_id:?}, got {:?}",
                        result.tool_call_id
                    ),
                ));
            }
        }
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

fn tool_call_id(index: usize) -> String {
    format!("ahp-test-tool-{}", index + 1)
}
