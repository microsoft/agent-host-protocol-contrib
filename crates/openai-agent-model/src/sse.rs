//! Bounded SSE framing and cancel-on-drop model event streams.

use std::mem;
use std::time::Duration;

use async_stream::stream;
use futures_util::StreamExt;
use tokio::time::{timeout, Instant};

use crate::chat::ChatStreamParser;
use crate::responses::ResponsesStreamParser;
use crate::{ModelApi, ModelError, ModelEventStream, ModelStreamEvent};

const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_EVENT_BYTES: usize = 256 * 1024;
const MAX_DATA_BYTES: usize = 192 * 1024;

pub(crate) struct SseEvent {
    pub(crate) event: Option<String>,
    pub(crate) data: String,
}

pub(crate) struct ParsedEvent {
    pub(crate) events: Vec<ModelStreamEvent>,
    pub(crate) terminal: bool,
}

pub(crate) fn response_stream(
    api: ModelApi,
    response: reqwest::Response,
    idle: Duration,
) -> ModelEventStream {
    Box::pin(stream! {
        let mut decoder = SseDecoder::default();
        let mut parser = match api {
            ModelApi::ChatCompletions => StreamParser::Chat(ChatStreamParser::default()),
            ModelApi::Responses => StreamParser::Responses(ResponsesStreamParser::default()),
            ModelApi::Auto => {
                yield Err(ModelError::InvalidResponse("Auto cannot decode a stream".to_owned()));
                return;
            }
        };
        let body = response.bytes_stream();
        futures_util::pin_mut!(body);
        loop {
            let deadline = Instant::now() + idle;
            let chunk = loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let next = match timeout(remaining, body.next()).await {
                    Ok(next) => next,
                    Err(_) => {
                        yield Err(ModelError::IdleTimeout { timeout: idle });
                        return;
                    }
                };
                let Some(chunk) = next else {
                    yield Err(ModelError::InvalidResponse(
                        "SSE stream ended before its terminal event".to_owned(),
                    ));
                    return;
                };
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        yield Err(ModelError::Request(error));
                        return;
                    }
                };
                if !chunk.is_empty() {
                    break chunk;
                }
            };
            let events = match decoder.push(&chunk) {
                Ok(events) => events,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };
            for event in events {
                let parsed = match parser.parse(event) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        yield Err(error);
                        return;
                    }
                };
                for event in parsed.events {
                    yield Ok(event);
                }
                if parsed.terminal {
                    yield Ok(ModelStreamEvent::Completed);
                    return;
                }
            }
        }
    })
}

enum StreamParser {
    Chat(ChatStreamParser),
    Responses(ResponsesStreamParser),
}

impl StreamParser {
    fn parse(&mut self, event: SseEvent) -> Result<ParsedEvent, ModelError> {
        match self {
            Self::Chat(parser) => parser.parse(event),
            Self::Responses(parser) => parser.parse(event),
        }
    }
}

#[derive(Default)]
struct SseDecoder {
    line: Vec<u8>,
    event_name: Option<String>,
    data_lines: Vec<String>,
    event_bytes: usize,
    data_bytes: usize,
}

impl SseDecoder {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, ModelError> {
        let mut events = Vec::new();
        for byte in bytes {
            self.line.push(*byte);
            if self.line.len() > MAX_LINE_BYTES {
                return Err(invalid("SSE line exceeds the configured size limit"));
            }
            if *byte == b'\n' {
                let mut line = mem::take(&mut self.line);
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if let Some(event) = self.parse_line(&line)? {
                    events.push(event);
                }
            }
        }
        Ok(events)
    }

    fn parse_line(&mut self, line: &[u8]) -> Result<Option<SseEvent>, ModelError> {
        if line.is_empty() {
            self.event_bytes = 0;
            self.data_bytes = 0;
            let data = self.data_lines.join("\n");
            self.data_lines.clear();
            let event_name = self.event_name.take();
            return Ok((!data.is_empty()).then_some(SseEvent {
                event: event_name,
                data,
            }));
        }
        self.event_bytes = self.event_bytes.saturating_add(line.len());
        if self.event_bytes > MAX_EVENT_BYTES {
            return Err(invalid("SSE event exceeds the configured size limit"));
        }
        if line.first() == Some(&b':') {
            return Ok(None);
        }
        let line =
            std::str::from_utf8(line).map_err(|_| invalid("SSE event contains non-UTF-8 data"))?;
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event_name = (!value.is_empty()).then(|| value.to_owned()),
            "data" => {
                self.data_bytes = self
                    .data_bytes
                    .saturating_add(value.len())
                    .saturating_add(usize::from(!self.data_lines.is_empty()));
                if self.data_bytes > MAX_DATA_BYTES {
                    return Err(invalid("SSE data exceeds the configured size limit"));
                }
                self.data_lines.push(value.to_owned());
            }
            _ => {}
        }
        Ok(None)
    }
}

fn invalid(message: &str) -> ModelError {
    ModelError::InvalidResponse(message.to_owned())
}

#[cfg(test)]
#[path = "sse_tests.rs"]
mod tests;
