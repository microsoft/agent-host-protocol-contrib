//! AHP result conversion for workspace tool execution.

use ahp_types::state::{ToolCallResult, ToolResultContent, ToolResultTextContent};
use serde_json::{json, Value};

/// A normalized result returned by a workspace tool.
#[derive(Clone, Debug)]
pub struct ToolExecution {
    /// Whether the tool completed successfully.
    pub success: bool,
    /// A past-tense summary suitable for AHP clients.
    pub past_tense_message: String,
    /// AHP content blocks describing the result.
    pub content: Vec<ToolResultContent>,
    /// Structured error details when `success` is false.
    pub error: Option<Value>,
}

impl ToolExecution {
    /// Builds a successful text result.
    #[must_use]
    pub fn succeeded(past_tense_message: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            success: true,
            past_tense_message: past_tense_message.into(),
            content: vec![ToolResultContent::Text(ToolResultTextContent {
                text: text.into(),
            })],
            error: None,
        }
    }

    /// Builds a failed text result with a message error object.
    #[must_use]
    pub fn failed(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            success: false,
            past_tense_message: "Tool call failed".into(),
            content: vec![ToolResultContent::Text(ToolResultTextContent {
                text: message.clone(),
            })],
            error: Some(json!({ "message": message })),
        }
    }

    /// Returns text content formatted for supplying back to a model.
    #[must_use]
    pub fn model_content(&self) -> String {
        let text = self
            .content
            .iter()
            .filter_map(|content| match content {
                ToolResultContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if self.success {
            text
        } else {
            format!("Tool failed: {text}")
        }
    }

    /// Converts this result to the AHP `ToolCallResult` shape.
    #[must_use]
    pub fn to_call_result(&self) -> ToolCallResult {
        ToolCallResult {
            success: self.success,
            past_tense_message: self.past_tense_message.clone().into(),
            content: Some(self.content.clone()),
            structured_content: None,
            error: self.error.clone(),
        }
    }
}

impl From<ToolExecution> for ToolCallResult {
    fn from(value: ToolExecution) -> Self {
        value.to_call_result()
    }
}
