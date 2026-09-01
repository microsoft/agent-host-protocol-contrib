//! Parser and state machine for immutable deterministic fixture v2.

use std::collections::HashSet;

use serde_json::Value;

use super::contract::{code, ContractError};
use super::v2_json::canonical_json;

const OPEN: &str = "[AHP-TEST v2]";
const CLOSE: &str = "[/AHP-TEST]";

pub(crate) struct V2FixtureSpec {
    pub(super) calls: Vec<V2ToolCall>,
    pub(super) response: String,
    pub(crate) delay_ms: u64,
}

pub(super) struct V2ToolCall {
    pub(super) name: String,
    pub(super) arguments: String,
}

pub(crate) enum V2Action {
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    FinalText(String),
}

pub(crate) fn has_marker(content: &str) -> bool {
    content.lines().any(|line| line == OPEN)
}

impl V2FixtureSpec {
    pub(crate) fn parse(content: &str) -> Result<Self, ContractError> {
        let lines = content.lines().collect::<Vec<_>>();
        let opens = marker_indexes(&lines, OPEN);
        let closes = marker_indexes(&lines, CLOSE);
        if opens.len() != 1 || closes.len() != 1 {
            return Err(ContractError::new(
                code::INVALID_DIRECTIVE,
                "exactly one complete-line [AHP-TEST v2] ... [/AHP-TEST] block is required",
            ));
        }
        let open = opens[0];
        let close = closes[0];
        if close <= open {
            return Err(ContractError::new(
                code::INVALID_DIRECTIVE,
                "the closing marker must occur after the opening marker",
            ));
        }
        if lines.iter().enumerate().any(|(index, line)| {
            index != open
                && index != close
                && (line.starts_with("[AHP-TEST") || line.starts_with("[/AHP-TEST"))
        }) {
            return Err(ContractError::new(
                code::INVALID_DIRECTIVE,
                "markers must be complete lines and may occur only once",
            ));
        }

        let mut calls = Vec::new();
        let mut response = None;
        let mut delay_ms = None;
        for line in lines[open + 1..close]
            .iter()
            .copied()
            .filter(|line| !line.is_empty())
        {
            if let Some(value) = line.strip_prefix("Call tool ") {
                if response.is_some() || delay_ms.is_some() {
                    return Err(ContractError::new(
                        code::INVALID_DIRECTIVE,
                        "tool-call directives must precede the final response directive",
                    ));
                }
                calls.push(parse_call(value)?);
            } else if let Some(value) = line.strip_prefix("Respond exactly after tools with JSON: ")
            {
                if response.replace(parse_final_response(value)?).is_some() {
                    return Err(ContractError::new(
                        code::INVALID_DIRECTIVE,
                        "AHP-TEST v2 has more than one final response directive",
                    ));
                }
            } else if let Some(value) = line
                .strip_prefix("Wait ")
                .and_then(|value| value.strip_suffix(" ms before every output chunk."))
            {
                if response.is_none() {
                    return Err(ContractError::new(
                        code::INVALID_DIRECTIVE,
                        "the delay directive must follow the final response directive",
                    ));
                }
                if delay_ms.replace(parse_delay(value)?).is_some() {
                    return Err(ContractError::new(
                        code::INVALID_DIRECTIVE,
                        "AHP-TEST v2 has more than one delay directive",
                    ));
                }
            } else {
                return Err(ContractError::new(
                    code::INVALID_DIRECTIVE,
                    format!("unknown or malformed AHP-TEST v2 directive {line:?}"),
                ));
            }
        }
        if calls.is_empty() {
            return Err(ContractError::new(
                code::INVALID_DIRECTIVE,
                "AHP-TEST v2 requires one or more tool-call directives",
            ));
        }
        let response = response.ok_or_else(|| {
            ContractError::new(
                code::INVALID_DIRECTIVE,
                "Respond exactly after tools with JSON: <json-string>. is required",
            )
        })?;
        Ok(Self {
            calls,
            response,
            delay_ms: delay_ms.unwrap_or(0),
        })
    }

    pub(crate) fn validate_tools(&self, tool_names: &HashSet<String>) -> Result<(), ContractError> {
        for call in &self.calls {
            if !tool_names.contains(&call.name) {
                return Err(ContractError::new(
                    code::UNKNOWN_TOOL,
                    format!(
                        "AHP-TEST v2 requested tool {:?}, which is not in request.tools",
                        call.name
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn marker_indexes(lines: &[&str], marker: &str) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (*line == marker).then_some(index))
        .collect()
}

fn parse_call(value: &str) -> Result<V2ToolCall, ContractError> {
    let (name, arguments) = value.split_once(" with JSON: ").ok_or_else(|| {
        ContractError::new(
            code::INVALID_DIRECTIVE,
            "tool-call directive must contain `Call tool <name> with JSON: <json-object>`",
        )
    })?;
    if !is_function_name(name) {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "tool names must contain 1..64 ASCII letters, digits, underscores, or hyphens",
        ));
    }
    if arguments.trim() != arguments {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "tool-call JSON must not have leading or trailing whitespace",
        ));
    }
    let value: Value = serde_json::from_str(arguments).map_err(|_| {
        ContractError::new(
            code::INVALID_DIRECTIVE,
            "tool-call JSON must be one complete JSON object",
        )
    })?;
    if !value.is_object() {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "tool-call JSON must be one complete JSON object",
        ));
    }
    Ok(V2ToolCall {
        name: name.to_owned(),
        arguments: canonical_json(&value)?,
    })
}

fn parse_final_response(value: &str) -> Result<String, ContractError> {
    let value = value.strip_suffix('.').ok_or_else(|| {
        ContractError::new(
            code::INVALID_DIRECTIVE,
            "final response directive must end with a period",
        )
    })?;
    if value.trim() != value {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "final response JSON must not have leading or trailing whitespace",
        ));
    }
    serde_json::from_str(value).map_err(|_| {
        ContractError::new(
            code::INVALID_DIRECTIVE,
            "final response JSON must be a complete JSON string literal",
        )
    })
}

fn parse_delay(value: &str) -> Result<u64, ContractError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "delay must be an ASCII decimal integer",
        ));
    }
    let delay = value
        .parse::<u64>()
        .map_err(|_| ContractError::new(code::INVALID_DIRECTIVE, "delay is too large"))?;
    if delay > 60_000 {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "delay must be in 0..60000 milliseconds",
        ));
    }
    Ok(delay)
}

fn is_function_name(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{code, V2FixtureSpec};

    #[test]
    fn parses_calls_and_canonicalizes_arguments() -> Result<(), String> {
        let spec = V2FixtureSpec::parse(
            "[AHP-TEST v2]\nCall tool weather with JSON: {\"z\":1,\"a\":[true,null]}\nRespond exactly after tools with JSON: \"done\".\nWait 4 ms before every output chunk.\n[/AHP-TEST]",
        )
        .map_err(|error| error.message)?;
        assert_eq!(spec.delay_ms, 4);
        assert!(spec
            .validate_tools(&HashSet::from(["weather".to_owned()]))
            .is_ok());
        Ok(())
    }

    #[test]
    fn rejects_invalid_v2_directives() {
        let cases = [
            "[AHP-TEST v2]\nRespond exactly after tools with JSON: \"done\".\n[/AHP-TEST]",
            "[AHP-TEST v2]\nCall tool weather with JSON: []\nRespond exactly after tools with JSON: \"done\".\n[/AHP-TEST]",
            "[AHP-TEST v2]\nCall tool weather with JSON: {}\nRespond exactly after tools with JSON: \"done\"\n[/AHP-TEST]",
            "[AHP-TEST v2]\nCall tool weather with JSON: {}\nStream in exactly 2 chunks.\nRespond exactly after tools with JSON: \"done\".\n[/AHP-TEST]",
        ];
        for prompt in cases {
            assert!(V2FixtureSpec::parse(prompt)
                .err()
                .is_some_and(|error| error.code == code::INVALID_DIRECTIVE));
        }
    }
}
