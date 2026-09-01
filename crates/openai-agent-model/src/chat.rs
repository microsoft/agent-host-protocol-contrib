//! Semantic validation for streamed Chat Completions chunks.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value};

use crate::chat_helpers::{empty, invalid, json_index, optional_non_empty_string, update_identity};
use crate::sse::{ParsedEvent, SseEvent};
use crate::{ModelError, ModelStreamEvent};

#[derive(Default)]
pub(crate) struct ChatStreamParser {
    calls: BTreeMap<usize, ChatToolCall>,
    ids: HashMap<String, usize>,
    finished: bool,
}

#[derive(Default)]
struct ChatToolCall {
    id: Option<String>,
    name: Option<String>,
    function_type: bool,
    started: bool,
}

impl ChatStreamParser {
    pub(crate) fn parse(&mut self, event: SseEvent) -> Result<ParsedEvent, ModelError> {
        if event.data == "[DONE]" {
            return self
                .finished
                .then_some(ParsedEvent {
                    events: Vec::new(),
                    terminal: true,
                })
                .ok_or_else(|| invalid("received [DONE] before a valid finish chunk"));
        }
        let value: Value = serde_json::from_str(&event.data)
            .map_err(|_| invalid("Chat Completions SSE data must be valid JSON"))?;
        let object = value
            .as_object()
            .ok_or_else(|| invalid("Chat Completions SSE data must be an object"))?;
        let Some(choices) = object.get("choices") else {
            return Ok(empty());
        };
        let choices = choices
            .as_array()
            .ok_or_else(|| invalid("Chat Completions choices must be an array"))?;
        let Some(choice) = choices.first() else {
            return Ok(empty());
        };
        if self.finished {
            return Err(invalid(
                "received a Chat Completions choice after its finish chunk",
            ));
        }
        let choice = choice
            .as_object()
            .ok_or_else(|| invalid("Chat Completions choice must be an object"))?;
        if let Some(index) = choice.get("index") {
            if json_index(index)? != 0 {
                return Err(invalid("only Chat Completions choice index 0 is supported"));
            }
        }
        let mut events = Vec::new();
        if let Some(delta) = choice.get("delta") {
            self.parse_delta(
                delta
                    .as_object()
                    .ok_or_else(|| invalid("Chat Completions delta must be an object"))?,
                &mut events,
            )?;
        }
        match choice.get("finish_reason") {
            None | Some(Value::Null) => {}
            Some(Value::String(reason)) if reason == "stop" || reason == "tool_calls" => {
                if self.calls.values().any(|call| !call.started) {
                    return Err(invalid(
                        "Chat Completions finished before a tool call had a valid id and name",
                    ));
                }
                if reason == "stop" && !self.calls.is_empty() {
                    return Err(invalid(
                        "Chat Completions stop finish cannot accompany function tool calls",
                    ));
                }
                if reason == "tool_calls" && self.calls.is_empty() {
                    return Err(invalid(
                        "Chat Completions tool_calls finish requires a function tool call",
                    ));
                }
                self.finished = true;
            }
            Some(Value::String(reason)) => {
                return Err(invalid(&format!(
                    "unsupported Chat Completions finish_reason {reason:?}"
                )));
            }
            Some(_) => {
                return Err(invalid(
                    "Chat Completions finish_reason must be a string or null",
                ));
            }
        }
        Ok(ParsedEvent {
            events,
            terminal: false,
        })
    }

    fn parse_delta(
        &mut self,
        delta: &Map<String, Value>,
        events: &mut Vec<ModelStreamEvent>,
    ) -> Result<(), ModelError> {
        if let Some(content) = delta.get("content") {
            match content {
                Value::String(content) if !content.is_empty() => {
                    events.push(ModelStreamEvent::TextDelta(content.clone()));
                }
                Value::String(_) | Value::Null => {}
                _ => {
                    return Err(invalid(
                        "Chat Completions delta content must be a string or null",
                    ));
                }
            }
        }
        if let Some(tool_calls) = delta.get("tool_calls") {
            for call in tool_calls
                .as_array()
                .ok_or_else(|| invalid("Chat Completions delta tool_calls must be an array"))?
            {
                self.parse_tool_call(
                    call.as_object().ok_or_else(|| {
                        invalid("Chat Completions tool call delta must be an object")
                    })?,
                    events,
                )?;
            }
        }
        Ok(())
    }

    fn parse_tool_call(
        &mut self,
        call: &Map<String, Value>,
        events: &mut Vec<ModelStreamEvent>,
    ) -> Result<(), ModelError> {
        let index = json_index(
            call.get("index")
                .ok_or_else(|| invalid("Chat Completions tool call delta requires an index"))?,
        )?;
        let id = optional_non_empty_string(call.get("id"), "Chat Completions tool call id")?;
        if let Some(id) = &id {
            if let Some(previous_index) = self.ids.get(id) {
                if *previous_index != index {
                    return Err(invalid("Chat Completions tool call id is duplicated"));
                }
            } else {
                self.ids.insert(id.clone(), index);
            }
        }
        let function = call
            .get("function")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                invalid("Chat Completions tool call delta requires a function object")
            })?;
        let name =
            optional_non_empty_string(function.get("name"), "Chat Completions function name")?;
        let arguments = match function.get("arguments") {
            Some(Value::String(arguments)) => Some(arguments),
            Some(_) => {
                return Err(invalid(
                    "Chat Completions function arguments delta must be a string",
                ));
            }
            None => None,
        };
        let function_type = match call.get("type") {
            Some(Value::String(value)) if value == "function" => true,
            Some(Value::String(_)) => {
                return Err(invalid(
                    "Chat Completions tool call delta type must be \"function\"",
                ));
            }
            Some(_) => {
                return Err(invalid(
                    "Chat Completions tool call delta type must be a string",
                ));
            }
            None => false,
        };
        let state = self.calls.entry(index).or_default();
        update_identity(&mut state.id, id, "Chat Completions tool call id")?;
        update_identity(&mut state.name, name, "Chat Completions function name")?;
        state.function_type |= function_type;
        if !state.started {
            if let (Some(id), Some(name), true) = (&state.id, &state.name, state.function_type) {
                state.started = true;
                events.push(ModelStreamEvent::ToolCallStarted {
                    index,
                    id: id.clone(),
                    name: name.clone(),
                });
            }
        }
        if let Some(arguments) = arguments.filter(|arguments| !arguments.is_empty()) {
            if !state.started {
                return Err(invalid(
                    "Chat Completions function arguments arrived before its call identity",
                ));
            }
            events.push(ModelStreamEvent::ToolCallArgumentsDelta {
                index,
                delta: arguments.clone(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
