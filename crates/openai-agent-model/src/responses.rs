//! Semantic validation for streamed OpenAI Responses API events.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value};

use crate::responses_helpers::{
    empty, event_type, failure_message, invalid, optional_string, output_index,
    required_non_empty_string, required_object, required_string,
};
use crate::sse::{ParsedEvent, SseEvent};
use crate::{ModelError, ModelStreamEvent};

#[derive(Default)]
pub(crate) struct ResponsesStreamParser {
    calls: BTreeMap<usize, ResponsesToolCall>,
    ids: HashMap<String, usize>,
}

#[derive(Default)]
struct ResponsesToolCall {
    id: String,
    name: String,
    arguments: String,
    arguments_done: bool,
    item_done: bool,
}

impl ResponsesStreamParser {
    pub(crate) fn parse(&mut self, event: SseEvent) -> Result<ParsedEvent, ModelError> {
        let value: Value = serde_json::from_str(&event.data)
            .map_err(|_| invalid("Responses SSE data must be valid JSON"))?;
        let object = value
            .as_object()
            .ok_or_else(|| invalid("Responses SSE data must be an object"))?;
        let event_type = event_type(&event, object)?;
        match event_type {
            "response.output_text.delta" => {
                let delta = required_string(object, "delta", event_type)?;
                Ok(ParsedEvent {
                    events: (!delta.is_empty())
                        .then(|| ModelStreamEvent::TextDelta(delta.to_owned()))
                        .into_iter()
                        .collect(),
                    terminal: false,
                })
            }
            "response.output_item.added" => self.output_item_added(object),
            "response.function_call_arguments.delta" => self.arguments_delta(object),
            "response.function_call_arguments.done" => self.arguments_done(object),
            "response.output_item.done" => self.output_item_done(object),
            "response.completed" => self.completed(),
            "response.failed" | "response.incomplete" | "error" => Err(ModelError::StreamFailed {
                event: event_type.to_owned(),
                message: failure_message(object),
            }),
            _ => Ok(empty()),
        }
    }

    fn output_item_added(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<ParsedEvent, ModelError> {
        let output_index = output_index(object)?;
        let item = required_object(object, "item", "response.output_item.added")?;
        if required_string(item, "type", "response.output_item.added.item")? != "function_call" {
            return Ok(empty());
        }
        if self.calls.contains_key(&output_index) {
            return Err(invalid(
                "Responses function call output_index was added more than once",
            ));
        }
        let id = required_non_empty_string(item, "call_id", "Responses function call")?;
        if self.ids.insert(id.clone(), output_index).is_some() {
            return Err(invalid("Responses function call call_id is duplicated"));
        }
        let name = required_non_empty_string(item, "name", "Responses function call")?;
        let arguments = optional_string(item, "arguments", "Responses function call")?
            .unwrap_or_default()
            .to_owned();
        self.calls.insert(
            output_index,
            ResponsesToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
                arguments_done: false,
                item_done: false,
            },
        );
        let mut events = vec![ModelStreamEvent::ToolCallStarted {
            index: output_index,
            id,
            name,
        }];
        if !arguments.is_empty() {
            events.push(ModelStreamEvent::ToolCallArgumentsDelta {
                index: output_index,
                delta: arguments,
            });
        }
        Ok(ParsedEvent {
            events,
            terminal: false,
        })
    }

    fn arguments_delta(&mut self, object: &Map<String, Value>) -> Result<ParsedEvent, ModelError> {
        let output_index = output_index(object)?;
        let delta = required_string(object, "delta", "Responses function arguments delta")?;
        let call = self.call_mut(output_index)?;
        if call.arguments_done {
            return Err(invalid(
                "Responses function arguments changed after their done event",
            ));
        }
        call.arguments.push_str(delta);
        Ok(ParsedEvent {
            events: (!delta.is_empty())
                .then(|| ModelStreamEvent::ToolCallArgumentsDelta {
                    index: output_index,
                    delta: delta.to_owned(),
                })
                .into_iter()
                .collect(),
            terminal: false,
        })
    }

    fn arguments_done(&mut self, object: &Map<String, Value>) -> Result<ParsedEvent, ModelError> {
        let output_index = output_index(object)?;
        let id = optional_string(object, "call_id", "Responses function arguments done")?;
        let name = optional_string(object, "name", "Responses function arguments done")?;
        let arguments = required_string(object, "arguments", "Responses function arguments done")?;
        let call = self.call_mut(output_index)?;
        if call.arguments_done
            || id.is_some_and(|id| id != call.id)
            || name.is_some_and(|name| name != call.name)
            || call.arguments != arguments
        {
            return Err(invalid(
                "Responses function arguments done event did not match streamed arguments",
            ));
        }
        call.arguments_done = true;
        Ok(empty())
    }

    fn output_item_done(&mut self, object: &Map<String, Value>) -> Result<ParsedEvent, ModelError> {
        let output_index = output_index(object)?;
        let item = required_object(object, "item", "response.output_item.done")?;
        if required_string(item, "type", "response.output_item.done.item")? != "function_call" {
            return Ok(empty());
        }
        let id = required_non_empty_string(item, "call_id", "Responses function call")?;
        let name = required_non_empty_string(item, "name", "Responses function call")?;
        let arguments = required_string(item, "arguments", "Responses function call")?;
        let call = self.call_mut(output_index)?;
        if call.item_done
            || !call.arguments_done
            || call.id != id
            || call.name != name
            || call.arguments != arguments
        {
            return Err(invalid(
                "Responses function call output item done event was semantically inconsistent",
            ));
        }
        call.item_done = true;
        Ok(empty())
    }

    fn completed(&self) -> Result<ParsedEvent, ModelError> {
        if self.calls.values().any(|call| !call.item_done) {
            return Err(invalid(
                "Responses completed before all function calls had completed lifecycle events",
            ));
        }
        Ok(ParsedEvent {
            events: Vec::new(),
            terminal: true,
        })
    }

    fn call_mut(&mut self, output_index: usize) -> Result<&mut ResponsesToolCall, ModelError> {
        self.calls.get_mut(&output_index).ok_or_else(|| {
            invalid("Responses function-call argument event referenced an unknown output_index")
        })
    }
}

#[cfg(test)]
#[path = "responses_tests.rs"]
mod tests;
