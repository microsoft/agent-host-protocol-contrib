//! Helpers for collecting a stream into one reply.

use std::collections::BTreeMap;

use futures_util::StreamExt;

use crate::{ModelError, ModelEventStream, ModelReply, ModelStreamEvent, ModelToolCall};

/// Collects a terminally valid stream into a single reply.
pub async fn collect_stream(mut stream: ModelEventStream) -> Result<ModelReply, ModelError> {
    let mut text = String::new();
    let mut saw_text = false;
    let mut calls = BTreeMap::<usize, CollectedCall>::new();
    let mut response_id = None;
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event? {
            ModelStreamEvent::ResponseStarted { id } => {
                if response_id.replace(id).is_some() {
                    return Err(ModelError::InvalidResponse(
                        "stream identified its response more than once".to_owned(),
                    ));
                }
            }
            ModelStreamEvent::TextDelta(delta) => {
                saw_text = true;
                text.push_str(&delta);
            }
            ModelStreamEvent::ToolCallStarted { index, id, name } => {
                if calls
                    .insert(
                        index,
                        CollectedCall {
                            id,
                            name,
                            arguments: String::new(),
                        },
                    )
                    .is_some()
                {
                    return Err(ModelError::InvalidResponse(
                        "stream started the same tool call index more than once".to_owned(),
                    ));
                }
            }
            ModelStreamEvent::ToolCallArgumentsDelta { index, delta } => {
                let call = calls.get_mut(&index).ok_or_else(|| {
                    ModelError::InvalidResponse(
                        "stream produced arguments before starting its tool call".to_owned(),
                    )
                })?;
                call.arguments.push_str(&delta);
            }
            ModelStreamEvent::Completed => {
                completed = true;
                break;
            }
        }
    }
    if !completed {
        return Err(ModelError::InvalidResponse(
            "stream ended without a completed event".to_owned(),
        ));
    }
    let tool_calls: Vec<ModelToolCall> = calls
        .into_values()
        .map(|call| ModelToolCall {
            id: call.id,
            name: call.name,
            arguments: call.arguments,
        })
        .collect();
    Ok(ModelReply {
        content: (saw_text || tool_calls.is_empty()).then_some(text),
        tool_calls,
        response_id,
    })
}

struct CollectedCall {
    id: String,
    name: String,
    arguments: String,
}
