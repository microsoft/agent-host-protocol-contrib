//! Conversion from durable AHP turns to OpenAI conversation messages.

use ahp_types::state::{ResponsePart, Turn};
use openai_agent_model::ModelMessage;

pub(super) fn model_messages(
    history: &[Turn],
    workspace: &str,
    current_user_text: &str,
) -> Vec<ModelMessage> {
    let mut messages = Vec::with_capacity(history.len().saturating_mul(2).saturating_add(1));
    for turn in history {
        messages.push(ModelMessage::User {
            content: turn.message.text.clone(),
        });
        if let Some(content) = assistant_text(&turn.response_parts) {
            messages.push(ModelMessage::Assistant {
                content: Some(content),
                tool_calls: Vec::new(),
            });
        }
    }
    messages.push(ModelMessage::User {
        content: format!(
            "The effective workspace is {workspace}. Use server tools only within that workspace.\n\n{current_user_text}"
        ),
    });
    messages
}

fn assistant_text(parts: &[ResponsePart]) -> Option<String> {
    let content = parts
        .iter()
        .filter_map(|part| match part {
            ResponsePart::Markdown(markdown) if !markdown.content.is_empty() => {
                Some(markdown.content.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!content.is_empty()).then_some(content)
}

#[cfg(test)]
mod tests {
    use ahp_types::state::Turn;
    use openai_agent_model::ModelMessage;
    use serde_json::json;

    use super::model_messages;

    #[test]
    fn includes_prior_users_and_ordered_assistant_markdown() -> Result<(), serde_json::Error> {
        let history = serde_json::from_value::<Vec<Turn>>(json!([{
            "id": "turn-1",
            "message": { "text": "What did you find?", "origin": { "kind": "user" } },
            "responseParts": [
                { "kind": "markdown", "id": "before", "content": "I checked." },
                {
                    "kind": "toolCall",
                    "toolCall": {
                        "status": "completed",
                        "toolCallId": "tool-1",
                        "toolName": "example",
                        "displayName": "Example",
                        "invocationMessage": "Running",
                        "success": true,
                        "pastTenseMessage": "Ran",
                        "confirmed": "not-needed"
                    }
                },
                { "kind": "markdown", "id": "after", "content": "The answer is 42." }
            ],
            "usage": null,
            "state": "complete"
        }]))?;

        assert_eq!(
            model_messages(&history, "file:///workspace", "What was your last response?"),
            vec![
                ModelMessage::User {
                    content: "What did you find?".into(),
                },
                ModelMessage::Assistant {
                    content: Some("I checked.\n\nThe answer is 42.".into()),
                    tool_calls: Vec::new(),
                },
                ModelMessage::User {
                    content: "The effective workspace is file:///workspace. Use server tools only within that workspace.\n\nWhat was your last response?".into(),
                },
            ]
        );
        Ok(())
    }
}
