use super::SseDecoder;
use crate::ModelError;

#[test]
fn handles_fragmented_crlf_comments_and_multiline_data() -> Result<(), String> {
    let mut decoder = SseDecoder::default();
    assert!(decoder
        .push(b": ignored\r\nevent: ex")
        .map_err(|error| error.to_string())?
        .is_empty());
    let events = decoder
        .push(b"ample\r\ndata: one\r\ndata: two\r\n\r\n")
        .map_err(|error| error.to_string())?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event.as_deref(), Some("example"));
    assert_eq!(events[0].data, "one\ntwo");
    Ok(())
}

#[test]
fn bounds_unterminated_sse_lines() {
    let mut decoder = SseDecoder::default();
    let oversized = vec![b'x'; 64 * 1024 + 1];
    assert!(matches!(
        decoder.push(&oversized),
        Err(ModelError::InvalidResponse(message)) if message.contains("line exceeds")
    ));
}
