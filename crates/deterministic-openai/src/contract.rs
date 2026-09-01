//! Shared deterministic directive data types and error codes.

use axum::http::StatusCode;

pub(crate) mod code {
    pub(crate) const INVALID_REQUEST: &str = "AHP_MOCK_INVALID_REQUEST";
    pub(crate) const UNKNOWN_TOOL: &str = "AHP_MOCK_UNKNOWN_TOOL";
    pub(crate) const INVALID_DIRECTIVE: &str = "AHP_TEST_INVALID_DIRECTIVE";
    pub(crate) const UNSUPPORTED_VERSION: &str = "AHP_TEST_UNSUPPORTED_VERSION";
    pub(crate) const UPSTREAM: &str = "AHP_TEST_UPSTREAM";
}

#[derive(Debug)]
pub(crate) struct ContractError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl ContractError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct UpstreamError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

/// Splits every Unicode scalar into exactly `requested` balanced chunks.
pub(crate) fn split_chunks(text: &str, requested: usize) -> Vec<String> {
    let characters = text.chars().collect::<Vec<_>>();
    let base = characters.len() / requested;
    let remainder = characters.len() % requested;
    let mut start = 0;
    (0..requested)
        .map(|index| {
            let length = base + usize::from(index < remainder);
            let result = characters[start..start + length].iter().collect();
            start += length;
            result
        })
        .collect()
}
