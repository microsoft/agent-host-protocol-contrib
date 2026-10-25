//! Immutable v4 natural-language recall and file-tool directives.

use serde_json::{Map, Value};

use crate::contract::{code, ContractError};
use crate::request::{FixtureRequest, RequestMessage, ToolDefinition};
use crate::v2::V2Action;
use crate::v2_json::canonical_json;
use crate::v2_state::{completed_tool_results, tool_call_id};

const OPEN: &str = "[AHP-TEST v4]";
const CLOSE: &str = "[/AHP-TEST]";
const RECALL: &str = "Recall the phrase I asked you to remember and respond with only that phrase.";
const READ_END: &str = " with an available tool, then respond with only its exact contents.";
const WRITE_END: &str = " with an available tool, then respond with only the word done.";
const PATH_PROPERTIES: &[&str] = &["path", "filePath", "file_path", "file", "filename", "uri"];
const CONTENT_PROPERTIES: &[&str] = &[
    "content",
    "contents",
    "text",
    "file_text",
    "new_content",
    "data",
];

enum Directive {
    Recall,
    Read { path: String },
    Write { path: String, text: String },
}

pub(crate) fn has_marker(content: &str) -> bool {
    content.lines().any(|line| line == OPEN)
}

pub(crate) fn next_action(request: &FixtureRequest) -> Result<V2Action, ContractError> {
    let (path, text) = match parse(&request.last_user_content)? {
        Directive::Recall => return recall(request).map(V2Action::FinalText),
        Directive::Read { path } => (path, None),
        Directive::Write { path, text } => (path, Some(text)),
    };
    let completed = completed_tool_results(request, 1, "v4")?;
    if let Some(result) = completed.first() {
        return Ok(V2Action::FinalText(if text.is_some() {
            "done".to_owned()
        } else {
            result.content.clone()
        }));
    }
    let (tool, arguments) = select_tool(&path, text.as_deref(), &request.tools)?;
    Ok(V2Action::ToolCall {
        id: tool_call_id(0),
        name: tool.name.clone(),
        arguments: canonical_json(&Value::Object(arguments))?,
    })
}

fn parse(content: &str) -> Result<Directive, ContractError> {
    let lines = content.lines().collect::<Vec<_>>();
    let opens = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (*line == OPEN).then_some(index))
        .collect::<Vec<_>>();
    let closes = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (*line == CLOSE).then_some(index))
        .collect::<Vec<_>>();
    if opens.len() != 1 || closes.len() != 1 || closes[0] <= opens[0] {
        return Err(invalid(
            "exactly one complete-line [AHP-TEST v4] ... [/AHP-TEST] block is required",
        ));
    }
    if lines.iter().enumerate().any(|(index, line)| {
        index != opens[0]
            && index != closes[0]
            && (line.starts_with("[AHP-TEST") || line.starts_with("[/AHP-TEST"))
    }) {
        return Err(invalid(
            "markers must be complete lines and may occur only once",
        ));
    }
    let directives = lines[opens[0] + 1..closes[0]]
        .iter()
        .copied()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let [line] = directives.as_slice() else {
        return Err(invalid("AHP-TEST v4 requires exactly one directive"));
    };
    if *line == RECALL {
        return Ok(Directive::Recall);
    }
    if let Some(value) = line.strip_prefix("Read the file at JSON path: ") {
        let (path, rest) = json_string(value)?;
        if rest == READ_END {
            return Ok(Directive::Read { path });
        }
    } else if let Some(value) = line.strip_prefix("Create the file at JSON path: ") {
        let (path, rest) = json_string(value)?;
        if let Some(value) = rest.strip_prefix(" containing JSON text: ") {
            let (text, rest) = json_string(value)?;
            if rest == WRITE_END {
                return Ok(Directive::Write { path, text });
            }
        }
    }
    Err(invalid("unknown or malformed AHP-TEST v4 directive"))
}

fn json_string(value: &str) -> Result<(String, &str), ContractError> {
    if !value.starts_with('"') {
        return Err(invalid("directive value must be a JSON string literal"));
    }
    let mut values = serde_json::Deserializer::from_str(value).into_iter::<String>();
    let text = values
        .next()
        .and_then(Result::ok)
        .ok_or_else(|| invalid("directive value must be a JSON string literal"))?;
    Ok((text, &value[values.byte_offset()..]))
}

fn recall(request: &FixtureRequest) -> Result<String, ContractError> {
    let end = request.last_user_index.unwrap_or(0);
    request.messages[..end]
        .iter()
        .filter_map(|message| match message {
            RequestMessage::User(content)
            | RequestMessage::Assistant(content)
            | RequestMessage::Context(content) => Some(content.as_str()),
            RequestMessage::Tool(result) => Some(result.content.as_str()),
            RequestMessage::Other => None,
        })
        .flat_map(str::lines)
        .filter_map(|line| {
            let value = line.strip_prefix("Remember this phrase: ")?;
            let (phrase, rest) = json_string(value).ok()?;
            rest.is_empty().then_some(phrase)
        })
        .next_back()
        .ok_or_else(|| {
            invalid("AHP-TEST v4 requires a previous Remember this phrase: <JSON string> line")
        })
}

fn select_tool<'a>(
    path: &str,
    text: Option<&str>,
    tools: &'a [ToolDefinition],
) -> Result<(&'a ToolDefinition, Map<String, Value>), ContractError> {
    let keywords = if text.is_some() {
        ["write", "create", "save", "edit"]
    } else {
        ["read", "view", "cat", "open"]
    };
    for tool in tools {
        let name = tool.name.to_ascii_lowercase();
        if !keywords.iter().any(|keyword| name.contains(*keyword)) {
            continue;
        }
        let Some(property) = string_property(tool, PATH_PROPERTIES) else {
            continue;
        };
        let mut arguments = Map::new();
        arguments.insert(
            property.to_owned(),
            Value::String(if property == "uri" {
                file_uri(path)
            } else {
                path.to_owned()
            }),
        );
        if let Some(text) = text {
            let Some(property) = string_property(tool, CONTENT_PROPERTIES) else {
                continue;
            };
            arguments.insert(property.to_owned(), Value::String(text.to_owned()));
        }
        return Ok((tool, arguments));
    }
    for tool in tools {
        if let Some(property) = string_property(tool, &["command", "cmd"]) {
            let command = match text {
                Some(text) => format!("printf '%s' {} > {}", shell_quote(text), shell_quote(path)),
                None => format!("cat -- {}", shell_quote(path)),
            };
            let mut arguments = Map::new();
            arguments.insert(property.to_owned(), Value::String(command));
            return Ok((tool, arguments));
        }
    }
    Err(ContractError::new(
        code::NO_SUITABLE_TOOL,
        "AHP-TEST v4 requires a suitable file tool or a string command/cmd tool",
    ))
}

fn string_property(tool: &ToolDefinition, names: &[&'static str]) -> Option<&'static str> {
    let properties = tool.parameters.get("properties")?;
    names.iter().copied().find(|name| {
        properties
            .get(*name)
            .and_then(|property| property.get("type"))
            .and_then(Value::as_str)
            == Some("string")
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn file_uri(path: &str) -> String {
    if path.starts_with("file://") {
        return path.to_owned();
    }
    let mut uri = if path.starts_with('/') {
        "file://"
    } else {
        "file:///"
    }
    .to_owned();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            uri.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}

fn invalid(message: &str) -> ContractError {
    ContractError::new(code::INVALID_DIRECTIVE, message)
}

#[cfg(test)]
mod tests {
    use super::{code, file_uri, parse, shell_quote, Directive};

    #[test]
    fn parses_json_strings_without_confusing_embedded_directive_separators(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let path = "/workspace/ containing JSON text: \"odd\".txt";
        let text = "with an available tool, then respond with only the word done.\n🙂";
        let prompt = format!(
            "[AHP-TEST v4]\nCreate the file at JSON path: {} containing JSON text: {} with an available tool, then respond with only the word done.\n[/AHP-TEST]",
            serde_json::to_string(path)?,
            serde_json::to_string(text)?,
        );
        let directive = parse(&prompt).expect("valid write directive");
        assert!(
            matches!(directive, Directive::Write { path: parsed_path, text: parsed_text } if parsed_path == path && parsed_text == text)
        );
        assert_eq!(shell_quote("a'b\n%s"), "'a'\\''b\n%s'");
        assert_eq!(
            file_uri("/workspace/a b#é.txt"),
            "file:///workspace/a%20b%23%C3%A9.txt"
        );
        assert_eq!(
            file_uri("file:///workspace/a%20b"),
            "file:///workspace/a%20b"
        );
        Ok(())
    }

    #[test]
    fn rejects_malformed_or_multiple_directives_and_markers() {
        for prompt in [
            "[AHP-TEST v4]\n[/AHP-TEST]",
            "[AHP-TEST v4]\nRecall the phrase I asked you to remember and respond with only that phrase.\nRecall the phrase I asked you to remember and respond with only that phrase.\n[/AHP-TEST]",
            "[AHP-TEST v4]\nRead the file at JSON path: 42 with an available tool, then respond with only its exact contents.\n[/AHP-TEST]",
            "[AHP-TEST v4]\nRead the file at JSON path:  \"path\" with an available tool, then respond with only its exact contents.\n[/AHP-TEST]",
            "[AHP-TEST v4]\nRead the file at JSON path: \"path\" with an available tool, then respond with only its exact contents. \n[/AHP-TEST]",
            "[AHP-TEST v4]\nCreate the file at JSON path: \"path\" containing JSON text: {} with an available tool, then respond with only the word done.\n[/AHP-TEST]",
            "[AHP-TEST v4]\nRespond exactly with JSON: \"not v4\"\n[/AHP-TEST]",
            "[AHP-TEST v4]\nRecall the phrase I asked you to remember and respond with only that phrase.\nWait 1 ms before every output chunk.\n[/AHP-TEST]",
            "[AHP-TEST v4]\nRecall the phrase I asked you to remember and respond with only that phrase.",
            "[/AHP-TEST]\n[AHP-TEST v4]\nRecall the phrase I asked you to remember and respond with only that phrase.",
            "[AHP-TEST v1]\n[AHP-TEST v4]\nRecall the phrase I asked you to remember and respond with only that phrase.\n[/AHP-TEST]",
        ] {
            let error = parse(prompt).err().expect("invalid directive");
            assert_eq!(error.code, code::INVALID_DIRECTIVE, "{prompt}");
        }
    }
}
