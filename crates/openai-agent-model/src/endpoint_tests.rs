use crate::endpoint::EndpointUrls;
use crate::ModelApi;

#[test]
fn derives_both_endpoints_or_forces_a_complete_endpoint() -> Result<(), String> {
    let endpoints = EndpointUrls::parse("https://example.test/prefix/v1/")
        .map_err(|error| error.to_string())?;
    assert_eq!(
        endpoints.url(ModelApi::Responses),
        "https://example.test/prefix/v1/responses"
    );
    assert_eq!(
        endpoints.url(ModelApi::ChatCompletions),
        "https://example.test/prefix/v1/chat/completions"
    );
    let forced = EndpointUrls::parse("https://example.test/v1/responses")
        .map_err(|error| error.to_string())?;
    assert_eq!(forced.forced_api(), Some(ModelApi::Responses));
    assert_eq!(
        forced.url(ModelApi::Responses),
        "https://example.test/v1/responses"
    );
    let forced_chat = EndpointUrls::parse("https://example.test/v1/chat/completions")
        .map_err(|error| error.to_string())?;
    assert_eq!(forced_chat.forced_api(), Some(ModelApi::ChatCompletions));
    assert_eq!(
        forced_chat.url(ModelApi::ChatCompletions),
        "https://example.test/v1/chat/completions"
    );
    Ok(())
}
