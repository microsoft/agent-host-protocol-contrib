//! Model rounds and stream consumption.

use std::collections::{BTreeMap, HashSet};
use std::str::FromStr;

use ahp_types::state::{ErrorInfo, MarkdownResponsePart, ModelSelection, ResponsePart};
use futures_util::StreamExt;
use openai_agent_model::{
    ModelError, ModelMessage, ModelRequest, ModelSelection as OpenAiModelSelection,
    ModelStreamEvent, ModelToolCall, ReasoningEffort,
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{
    cancelled,
    history::model_messages,
    split_scalars,
    tool::{append_response_part, append_text, append_tool_delta, execute_tool, start_tool},
    TurnFailure, TurnOutcome, MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_CALLS_PER_TURN,
};
use crate::host::{CoreHost, TurnKey};
use crate::presentation::ToolPresentation;
use crate::store::FinishTurn;

pub(crate) async fn run_turn(
    host: CoreHost,
    started: crate::StartedTurn,
    cancellation: CancellationToken,
) {
    let outcome = run_turn_inner(&host, &started, &cancellation).await;
    if !cancelled(&host, &cancellation) {
        let terminal = match outcome {
            TurnOutcome::Complete { model_cursor } => Some(FinishTurn::Complete {
                chat: started.chat.clone(),
                turn_id: started.turn_id.clone(),
                model_cursor,
            }),
            TurnOutcome::Failed(failure) => Some(FinishTurn::Error {
                chat: started.chat.clone(),
                turn_id: started.turn_id.clone(),
                error: ErrorInfo {
                    error_type: failure.error_type.into(),
                    message: failure.message,
                    stack: None,
                    meta: Some(serde_json::Map::from_iter([(
                        "code".into(),
                        Value::String(failure.code.into()),
                    )])),
                },
            }),
            TurnOutcome::Cancelled => None,
        };
        if let Some(terminal) = terminal {
            let _guard = host.inner.publish_lock.lock().await;
            if !cancelled(&host, &cancellation) {
                if let Ok(transition) = host.inner.store.finish_turn(terminal) {
                    host.fanout(&transition).await;
                }
            }
        }
    }
    host.inner.cancellations.lock().await.remove(&TurnKey {
        chat: started.chat,
        turn_id: started.turn_id,
    });
}

async fn run_turn_inner(
    host: &CoreHost,
    started: &crate::StartedTurn,
    cancellation: &CancellationToken,
) -> TurnOutcome {
    let workspace = match host
        .inner
        .profile
        .select_workspace(Some(vec![started.workspace.clone()]))
    {
        Ok(workspace) => workspace,
        Err(error) => {
            return TurnOutcome::Failed(TurnFailure::tool(format!(
                "session workspace is no longer valid: {error}"
            )))
        }
    };
    let chain_scope = host.inner.model.response_chain_scope().map(str::to_owned);
    let messages = model_messages(
        &started.history,
        started.workspace.as_str(),
        &started.message.text,
    );
    let current_message_index = messages.len().saturating_sub(1);
    let mut request = model_request_with_selection(
        ModelRequest {
            messages,
            tools: host.inner.profile.model_tools(),
            model_selection: None,
            response_continuation: None,
            store_response: chain_scope.is_some(),
        },
        &started.model_selection,
    );
    if let (Some(scope), Some(cursor)) = (&chain_scope, &started.model_cursor) {
        let cursor_is_current = started
            .history
            .last()
            .is_some_and(|turn| turn.id == cursor.turn_id);
        if cursor.scope == *scope && cursor_is_current {
            request = request
                .with_response_continuation(cursor.response_id.clone(), current_message_index);
        }
    }
    let mut seen_ids = HashSet::new();
    let mut total_calls = 0_usize;
    let mut next_text_segment = 1_usize;
    let mut use_initial_text_part = true;
    loop {
        let reply = match next_reply(
            host,
            started,
            &request,
            cancellation,
            use_initial_text_part,
            &mut next_text_segment,
        )
        .await
        {
            Ok(reply) => reply,
            Err(outcome) => return outcome,
        };
        if reply.tool_calls.is_empty() {
            let model_cursor = chain_scope.as_ref().and_then(|scope| {
                reply.response_id.map(|response_id| crate::ModelCursor {
                    scope: scope.clone(),
                    response_id,
                    turn_id: started.turn_id.clone(),
                })
            });
            return TurnOutcome::Complete { model_cursor };
        }

        if total_calls.saturating_add(reply.tool_calls.len()) > MAX_TOOL_CALLS_PER_TURN {
            return TurnOutcome::Failed(TurnFailure::tool(
                "model exceeded the maximum of eight tool calls per turn",
            ));
        }
        for call in &reply.tool_calls {
            if call.id.trim().is_empty() || !seen_ids.insert(call.id.clone()) {
                return TurnOutcome::Failed(TurnFailure::tool(
                    "model returned an empty or repeated tool call ID",
                ));
            }
        }
        total_calls += reply.tool_calls.len();
        use_initial_text_part = false;
        let response_id = reply.response_id.clone();
        request.messages.push(ModelMessage::Assistant {
            content: reply.content,
            tool_calls: reply.tool_calls.clone(),
        });
        let continuation_start = request.messages.len();
        for call in reply.tool_calls {
            let (tool_call_id, content) =
                match execute_tool(host, started, &workspace, call, cancellation).await {
                    Ok(value) => value,
                    Err(outcome) => return outcome,
                };
            request.messages.push(ModelMessage::Tool {
                tool_call_id,
                content,
            });
        }
        request.response_continuation =
            chain_scope
                .as_ref()
                .zip(response_id)
                .map(
                    |(_, response_id)| openai_agent_model::ResponseContinuation {
                        previous_response_id: response_id,
                        input_start: continuation_start,
                    },
                );
    }
}

fn model_request_with_selection(request: ModelRequest, selection: &ModelSelection) -> ModelRequest {
    let reasoning_effort = selection
        .config
        .as_ref()
        .and_then(|config| config.get("thinkingLevel"))
        .and_then(Value::as_str)
        .and_then(|level| ReasoningEffort::from_str(level).ok());
    request.with_model_selection(OpenAiModelSelection {
        id: selection.id.clone(),
        reasoning_effort,
    })
}

async fn next_reply(
    host: &CoreHost,
    started: &crate::StartedTurn,
    request: &ModelRequest,
    cancellation: &CancellationToken,
    use_initial_text_part: bool,
    next_text_segment: &mut usize,
) -> Result<openai_agent_model::ModelReply, TurnOutcome> {
    let opening = host.inner.model.stream(request);
    tokio::pin!(opening);
    let mut stream = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(TurnOutcome::Cancelled),
        _ = host.inner.shutdown.token().cancelled() => return Err(TurnOutcome::Cancelled),
        result = &mut opening => result.map_err(TurnFailure::model).map_err(TurnOutcome::Failed)?,
    };
    let mut text = String::new();
    let mut calls = BTreeMap::<usize, ModelToolCall>::new();
    let mut response_id = None;
    let mut complete = false;
    let mut text_part_id = use_initial_text_part.then(|| response_part_id(&started.turn_id, 0));
    loop {
        let next = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(TurnOutcome::Cancelled),
            _ = host.inner.shutdown.token().cancelled() => return Err(TurnOutcome::Cancelled),
            next = stream.next() => next,
        };
        let Some(next) = next else {
            break;
        };
        match next
            .map_err(TurnFailure::model)
            .map_err(TurnOutcome::Failed)?
        {
            ModelStreamEvent::ResponseStarted { id } => {
                if response_id.replace(id).is_some() {
                    return Err(TurnOutcome::Failed(TurnFailure::model(
                        ModelError::InvalidResponse(
                            "model stream identified its response more than once".into(),
                        ),
                    )));
                }
            }
            ModelStreamEvent::TextDelta(delta) => {
                if delta.is_empty() {
                    continue;
                }
                let part_id = match &text_part_id {
                    Some(part_id) => part_id.clone(),
                    None => {
                        let part_id = response_part_id(&started.turn_id, *next_text_segment);
                        *next_text_segment = next_text_segment.saturating_add(1);
                        append_response_part(
                            host,
                            started,
                            ResponsePart::Markdown(MarkdownResponsePart {
                                id: part_id.clone(),
                                content: String::new(),
                            }),
                            cancellation,
                        )
                        .await?;
                        text_part_id = Some(part_id.clone());
                        part_id
                    }
                };
                for chunk in split_scalars(&delta, host.inner.config.max_delta_scalars) {
                    append_text(host, started, part_id.clone(), chunk, cancellation).await?;
                }
                text.push_str(&delta);
            }
            ModelStreamEvent::ToolCallStarted { index, id, name } => {
                if calls.contains_key(&index) {
                    return Err(TurnOutcome::Failed(TurnFailure::tool(
                        "model repeated a tool call index",
                    )));
                }
                text_part_id = None;
                let presentation = ToolPresentation::new(id.clone(), name.clone(), String::new())
                    .map_err(TurnFailure::tool)
                    .map_err(TurnOutcome::Failed)?;
                start_tool(host, started, &presentation, cancellation).await?;
                calls.insert(
                    index,
                    ModelToolCall {
                        id,
                        name,
                        arguments: String::new(),
                    },
                );
            }

            ModelStreamEvent::ToolCallArgumentsDelta { index, delta } => {
                let Some(call) = calls.get_mut(&index) else {
                    return Err(TurnOutcome::Failed(TurnFailure::tool(
                        "model sent tool arguments before tool start",
                    )));
                };
                if call.arguments.len().saturating_add(delta.len()) > MAX_TOOL_ARGUMENT_BYTES {
                    return Err(TurnOutcome::Failed(TurnFailure::tool(
                        "model tool arguments exceeded 64 KiB",
                    )));
                }
                call.arguments.push_str(&delta);
                let presentation =
                    ToolPresentation::new(call.id.clone(), call.name.clone(), String::new())
                        .map_err(TurnFailure::tool)
                        .map_err(TurnOutcome::Failed)?;
                append_tool_delta(host, started, &presentation, delta, cancellation).await?;
            }
            ModelStreamEvent::Completed => {
                complete = true;
                break;
            }
        }
    }
    if !complete {
        return Err(TurnOutcome::Failed(TurnFailure::model(
            ModelError::InvalidResponse("model stream ended before completion".into()),
        )));
    }
    if calls.values().any(|call| {
        call.id.trim().is_empty()
            || call.name.trim().is_empty()
            || serde_json::from_str::<Value>(&call.arguments).is_err()
    }) {
        return Err(TurnOutcome::Failed(TurnFailure::tool(
            "model stream contained malformed tool call arguments",
        )));
    }
    Ok(openai_agent_model::ModelReply {
        content: (!text.is_empty()).then_some(text),
        tool_calls: calls.into_values().collect(),
        response_id,
    })
}

fn response_part_id(turn_id: &str, segment: usize) -> String {
    if segment == 0 {
        format!("response-{turn_id}")
    } else {
        format!("response-{turn_id}-{segment}")
    }
}
