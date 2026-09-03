//! Immutable v3 deterministic conversation-history grammar.

use crate::contract::{code, ContractError};
use crate::request::FixtureRequest;

const OPEN: &str = "[AHP-TEST v3]";
const CLOSE: &str = "[/AHP-TEST]";
const REPEAT: &str = "Repeat the previous assistant response exactly.";

pub(crate) fn has_marker(content: &str) -> bool {
    content.lines().any(|line| line == OPEN)
}

pub(crate) fn previous_response(request: &FixtureRequest) -> Result<String, ContractError> {
    let lines = request.last_user_content.lines().collect::<Vec<_>>();
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
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "exactly one complete-line [AHP-TEST v3] ... [/AHP-TEST] block is required",
        ));
    }
    let directives = lines[opens[0] + 1..closes[0]]
        .iter()
        .copied()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if directives != [REPEAT] {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            format!("AHP-TEST v3 requires the single directive {REPEAT:?}"),
        ));
    }
    request
        .previous_assistant_content()
        .filter(|content| !content.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ContractError::new(
                code::INVALID_REQUEST,
                "AHP-TEST v3 requires a previous assistant text message",
            )
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::request::FixtureRequest;

    #[test]
    fn repeats_history_for_both_request_shapes() {
        let prompt = "[AHP-TEST v3]\nRepeat the previous assistant response exactly.\n[/AHP-TEST]";
        for request in [
            FixtureRequest::parse_chat(json!({
                "model": "fixture-model",
                "messages": [
                    { "role": "user", "content": "first" },
                    { "role": "assistant", "content": "remembered" },
                    { "role": "user", "content": prompt }
                ]
            }))
            .expect("valid Chat Completions request"),
            FixtureRequest::parse_responses(json!({
                "model": "fixture-model",
                "input": [
                    { "role": "user", "content": "first" },
                    { "role": "assistant", "content": "remembered" },
                    { "role": "user", "content": prompt }
                ]
            }))
            .expect("valid Responses request"),
        ] {
            assert_eq!(
                super::previous_response(&request).expect("previous response"),
                "remembered"
            );
        }
    }
}
