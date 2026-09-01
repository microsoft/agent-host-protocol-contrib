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
    assert_eq!(endpoints.models(), "https://example.test/prefix/v1/models");
    let forced = EndpointUrls::parse("https://example.test/v1/responses")
        .map_err(|error| error.to_string())?;
    assert_eq!(forced.forced_api(), Some(ModelApi::Responses));
    assert_eq!(
        forced.url(ModelApi::Responses),
        "https://example.test/v1/responses"
    );
    assert_eq!(forced.models(), "https://example.test/v1/models");
    let forced_chat = EndpointUrls::parse("https://example.test/v1/chat/completions")
        .map_err(|error| error.to_string())?;
    assert_eq!(forced_chat.forced_api(), Some(ModelApi::ChatCompletions));
    assert_eq!(
        forced_chat.url(ModelApi::ChatCompletions),
        "https://example.test/v1/chat/completions"
    );
    assert_eq!(forced_chat.models(), "https://example.test/v1/models");
    Ok(())
}

#[test]
fn derives_model_catalog_from_roots_and_complete_endpoints() -> Result<(), String> {
    for (base_url, expected) in [
        ("https://example.test", "https://example.test/v1/models"),
        ("https://example.test/v1", "https://example.test/v1/models"),
        (
            "https://example.test/responses",
            "https://example.test/v1/models",
        ),
        (
            "https://example.test/chat/completions",
            "https://example.test/v1/models",
        ),
        (
            "https://example.test/v1/responses",
            "https://example.test/v1/models",
        ),
        (
            "https://example.test/v1/chat/completions",
            "https://example.test/v1/models",
        ),
    ] {
        let endpoints = EndpointUrls::parse(base_url).map_err(|error| error.to_string())?;
        assert_eq!(endpoints.models(), expected, "base URL {base_url}");
    }
    Ok(())
}
