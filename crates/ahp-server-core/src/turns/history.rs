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

pub(super) fn model_messages_with_attachments(
    started: &crate::StartedTurn,
) -> (Vec<ModelMessage>, usize) {
    let base = model_messages(&started.history, &started.workspace, &started.message.text);
    let mut base = base.into_iter();
    let mut messages = Vec::new();
    for turn in &started.history {
        if let Some(context) = attachment_message(
            &turn.message,
            started
                .materialized_attachments
                .get(&turn.id)
                .map(Vec::as_slice),
        ) {
            messages.push(context);
        }
        if let Some(user) = base.next() {
            messages.push(user);
        }
        if assistant_text(&turn.response_parts).is_some() {
            if let Some(assistant) = base.next() {
                messages.push(assistant);
            }
        }
    }
    let current_input_start = messages.len();
    if let Some(context) = attachment_message(
        &started.message,
        started
            .materialized_attachments
            .get(&started.turn_id)
            .map(Vec::as_slice),
    ) {
        messages.push(context);
    }
    messages.extend(base);
    (messages, current_input_start)
}

pub(super) fn attachment_message(
    message: &ahp_types::state::Message,
    context: Option<&[Turn]>,
) -> Option<ModelMessage> {
    let references = message
        .attachments
        .as_ref()?
        .iter()
        .filter(|attachment| matches!(attachment, ahp_types::state::MessageAttachment::Chat(_)))
        .collect::<Vec<_>>();
    if references.is_empty() {
        return None;
    }
    let mut content = format!(
        "Attached chat references: {}\nFixed transcript at message acceptance:\n",
        serde_json::to_string(&references).unwrap_or_default()
    );
    for turn in context.unwrap_or_default() {
        content.push_str("\nUser:\n");
        content.push_str(&turn.message.text);
        content.push('\n');
        if let Some(attachments) = &turn.message.attachments {
            // Nested attachments stay opaque references, not recursively materialized context.
            content.push_str("Attachment references: ");
            content.push_str(&serde_json::to_string(attachments).unwrap_or_default());
            content.push('\n');
        }
        for part in &turn.response_parts {
            if let ResponsePart::Markdown(markdown) = part {
                content.push_str("Assistant:\n");
                content.push_str(&markdown.content);
            } else {
                content.push_str("Response part: ");
                content.push_str(&serde_json::to_string(part).unwrap_or_default());
            }
            content.push('\n');
        }
    }
    Some(ModelMessage::User { content })
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
    fn attached_transcripts_reach_the_model_as_unescaped_message_text(
    ) -> Result<(), serde_json::Error> {
        let turns = serde_json::from_value::<Vec<Turn>>(json!([{
            "id":"source", "state":"complete", "responseParts":[],
            "message":{"text":"Remember this phrase: \"pinned phrase\"", "origin":{"kind":"user"},
                "attachments":[{"type":"chat","resource":"ahp-chat:/nested","label":"Opaque nested reference"}]}
        }]))?;
        let source = serde_json::from_value(
            json!({"text":"Recall the phrase", "origin":{"kind":"user"}, "attachments":[{"type":"chat","resource":"ahp-chat:/source","label":"Source"}]}),
        )?;
        let message = super::attachment_message(&source, Some(&turns));
        let content = match message {
            Some(ModelMessage::User { content }) => content,
            _ => String::new(),
        };
        assert!(content
            .lines()
            .any(|line| line == "Remember this phrase: \"pinned phrase\""));
        assert!(content.contains("ahp-chat:/nested"));
        let started = crate::StartedTurn {
            chat: "ahp-chat:/target".into(),
            session: "ahp-session:/target".into(),
            turn_id: "current".into(),
            history: Vec::new(),
            model_cursor: None,
            client_tools: Vec::new(),
            materialized_attachments: std::collections::BTreeMap::from([("current".into(), turns)]),
            message: source,
            workspace: "file:///workspace".into(),
            model_selection: crate::ModelCatalog::single("test")
                .default_selection
                .expect("default model"),
        };
        let (messages, input_start) = super::model_messages_with_attachments(&started);
        assert_eq!(input_start, 0);
        assert_eq!(messages.len(), 2);
        assert!(
            matches!(&messages[0], ModelMessage::User { content } if content.lines().any(|line| line == "Remember this phrase: \"pinned phrase\""))
        );
        assert!(
            matches!(&messages[1], ModelMessage::User { content } if content.ends_with("Recall the phrase"))
        );
        Ok(())
    }

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
