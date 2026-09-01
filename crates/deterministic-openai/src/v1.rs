//! Immutable v1 deterministic behavior grammar.

use crate::contract::{code, ContractError, UpstreamError};
use axum::http::StatusCode;

pub(crate) struct FixtureSpec {
    pub(crate) response: String,
    pub(crate) chunks: usize,
    pub(crate) delay_ms: u64,
    pub(crate) upstream_error: Option<UpstreamError>,
}

impl FixtureSpec {
    pub(crate) fn parse(last_user_content: &str) -> Result<Self, ContractError> {
        const OPEN: &str = "[AHP-TEST v1]";
        const CLOSE: &str = "[/AHP-TEST]";
        let lines = last_user_content.lines().collect::<Vec<_>>();
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
        let has_marker_candidate = lines
            .iter()
            .any(|line| line.starts_with("[AHP-TEST") || line.starts_with("[/AHP-TEST"));
        if opens.is_empty() && closes.is_empty() && !has_marker_candidate {
            return Ok(Self {
                response: format!("Deterministic echo: {last_user_content}"),
                chunks: 1,
                delay_ms: 0,
                upstream_error: None,
            });
        }
        if lines
            .iter()
            .any(|line| line.starts_with("[AHP-TEST v") && *line != OPEN && line.ends_with(']'))
        {
            return Err(ContractError::new(
                code::UNSUPPORTED_VERSION,
                "AHP-TEST v1 is the only supported directive version",
            ));
        }
        if opens.len() != 1 || closes.len() != 1 {
            return Err(ContractError::new(
                code::INVALID_DIRECTIVE,
                "exactly one complete-line [AHP-TEST v1] ... [/AHP-TEST] block is required",
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

        let mut response = None;
        let mut chunks = None;
        let mut delay_ms = None;
        let mut upstream_error = None;
        for line in lines[open + 1..close]
            .iter()
            .copied()
            .filter(|line| !line.is_empty())
        {
            if let Some(value) = line.strip_prefix("Respond exactly with JSON: ") {
                set_once(&mut response, parse_json_string(value)?, "response")?;
            } else if let Some(value) = line
                .strip_prefix("Stream in exactly ")
                .and_then(|value| value.strip_suffix(" chunks."))
            {
                set_once(&mut chunks, parse_chunks(value)?, "chunk count")?;
            } else if let Some(value) = line
                .strip_prefix("Wait ")
                .and_then(|value| value.strip_suffix(" ms before every output chunk."))
            {
                set_once(&mut delay_ms, parse_delay(value)?, "delay")?;
            } else if let Some(value) = line.strip_prefix("Return upstream HTTP ") {
                set_once(
                    &mut upstream_error,
                    parse_upstream_error(value)?,
                    "upstream error",
                )?;
            } else {
                return Err(ContractError::new(
                    code::INVALID_DIRECTIVE,
                    format!("unknown or malformed AHP-TEST v1 directive {line:?}"),
                ));
            }
        }
        if response.is_some() && upstream_error.is_some() {
            return Err(ContractError::new(
                code::INVALID_DIRECTIVE,
                "success and upstream-error directives are mutually exclusive",
            ));
        }
        if upstream_error.is_some() && (chunks.is_some() || delay_ms.is_some()) {
            return Err(ContractError::new(
                code::INVALID_DIRECTIVE,
                "stream and delay directives cannot accompany an upstream error",
            ));
        }
        let Some(response) = response else {
            if upstream_error.is_none() {
                return Err(ContractError::new(
                    code::INVALID_DIRECTIVE,
                    "Respond exactly with JSON: <json-string> is required for success",
                ));
            }
            return Ok(Self {
                response: String::new(),
                chunks: 1,
                delay_ms: 0,
                upstream_error,
            });
        };
        let chunks = chunks.unwrap_or(1);
        let scalar_count = response.chars().count();
        if (scalar_count == 0 && chunks != 1) || (scalar_count > 0 && chunks > scalar_count) {
            return Err(ContractError::new(
                code::INVALID_DIRECTIVE,
                "chunk count must not exceed the response Unicode-scalar length",
            ));
        }
        Ok(Self {
            response,
            chunks,
            delay_ms: delay_ms.unwrap_or(0),
            upstream_error: None,
        })
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), ContractError> {
    if slot.replace(value).is_some() {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            format!("AHP-TEST v1 has more than one {name} directive"),
        ));
    }
    Ok(())
}

fn parse_json_string(value: &str) -> Result<String, ContractError> {
    if value.trim() != value {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "directive values must not have leading or trailing whitespace",
        ));
    }
    serde_json::from_str(value).map_err(|_| {
        ContractError::new(
            code::INVALID_DIRECTIVE,
            "directive value must be a complete JSON string literal",
        )
    })
}

fn parse_chunks(value: &str) -> Result<usize, ContractError> {
    let count = parse_ascii_decimal(value, "chunk count")?
        .try_into()
        .map_err(|_| ContractError::new(code::INVALID_DIRECTIVE, "chunk count is too large"))?;
    if !(1..=1024).contains(&count) {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "chunk count must be in 1..1024",
        ));
    }
    Ok(count)
}

fn parse_delay(value: &str) -> Result<u64, ContractError> {
    let delay = parse_ascii_decimal(value, "delay")?;
    if delay > 60_000 {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "delay must be in 0..60000 milliseconds",
        ));
    }
    Ok(delay)
}

fn parse_upstream_error(value: &str) -> Result<UpstreamError, ContractError> {
    let value = value.strip_suffix('.').ok_or_else(|| {
        ContractError::new(
            code::INVALID_DIRECTIVE,
            "upstream-error directive must end with a period",
        )
    })?;
    let (status, message) = value.split_once(" with JSON message: ").ok_or_else(|| {
        ContractError::new(
            code::INVALID_DIRECTIVE,
            "upstream-error directive must contain `with JSON message: <json-string>.`",
        )
    })?;
    let status = parse_ascii_decimal(status, "upstream status")?
        .try_into()
        .map_err(|_| ContractError::new(code::INVALID_DIRECTIVE, "upstream status is too large"))?;
    if !(400..=599).contains(&status) {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            "upstream status must be in 400..599",
        ));
    }
    let status = StatusCode::from_u16(status)
        .map_err(|_| ContractError::new(code::INVALID_DIRECTIVE, "upstream status is invalid"))?;
    Ok(UpstreamError {
        status,
        message: parse_json_string(message)?,
    })
}

fn parse_ascii_decimal(value: &str, field: &str) -> Result<u64, ContractError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ContractError::new(
            code::INVALID_DIRECTIVE,
            format!("{field} must be an ASCII decimal integer"),
        ));
    }
    value
        .parse::<u64>()
        .map_err(|_| ContractError::new(code::INVALID_DIRECTIVE, format!("{field} is too large")))
}
