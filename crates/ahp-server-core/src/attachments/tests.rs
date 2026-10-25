use std::cell::Cell;

use ahp_types::state::{Message, Turn};
use serde_json::{json, Value};

use super::{materialize_attachments, MAX_CHAT_ATTACHMENTS, MAX_MATERIALIZED_BYTES};
use crate::{StoreConflict, StoreError};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn message(attachments: Vec<Value>) -> Result<Message> {
    Ok(serde_json::from_value(json!({
        "text": "Attached context", "origin": {"kind": "user"}, "attachments": attachments
    }))?)
}

fn attachment(resource: &str, end: Option<&str>) -> Value {
    let mut value = json!({"type": "chat", "resource": resource, "label": "Source"});
    if let Some(end) = end {
        value["endTurn"] = json!(end);
    }
    value
}

fn turn() -> Result<Turn> {
    Ok(serde_json::from_value(json!({
        "id": "retained", "state": "complete", "responseParts": [],
        "message": {"text": "", "origin": {"kind": "user"}}
    }))?)
}

#[test]
fn duplicate_resource_and_bound_is_resolved_and_retained_once() -> Result {
    let source = turn()?;
    let message = message(vec![
        attachment("ahp-chat:/source", None),
        attachment("ahp-chat:/source", None),
        attachment("ahp-chat:/source", Some("retained")),
        attachment("ahp-chat:/source", Some("retained")),
    ])?;
    let calls = Cell::new(0);
    let context = materialize_attachments(&message, |_| {
        calls.set(calls.get() + 1);
        Ok(vec![source.clone()])
    })?;
    assert_eq!(calls.get(), 2);
    assert_eq!(context, vec![source.clone(), source]);
    Ok(())
}

#[test]
fn raw_attachment_count_is_capped_before_any_transcript_is_copied() -> Result {
    let message = message(vec![
        attachment("ahp-chat:/source", None);
        MAX_CHAT_ATTACHMENTS + 1
    ])?;
    let calls = Cell::new(0);
    let result = materialize_attachments(&message, |_| {
        calls.set(calls.get() + 1);
        Ok(Vec::new())
    });
    assert!(matches!(
        result,
        Err(StoreError::Conflict(
            StoreConflict::ChatAttachmentLimitExceeded(_)
        ))
    ));
    assert_eq!(calls.get(), 0);
    Ok(())
}

#[test]
fn materialized_byte_budget_covers_all_unique_transcripts() -> Result {
    let mut source = turn()?;
    let overhead = serde_json::to_vec(&vec![source.clone()])?.len();
    source.message.text = "x".repeat(MAX_MATERIALIZED_BYTES - overhead);
    let single = message(vec![attachment("ahp-chat:/first", None)])?;
    let context = materialize_attachments(&single, |_| Ok(vec![source.clone()]))?;
    assert_eq!(serde_json::to_vec(&context)?.len(), MAX_MATERIALIZED_BYTES);

    let multiple = message(vec![
        attachment("ahp-chat:/first", None),
        attachment("ahp-chat:/second", None),
    ])?;
    let result = materialize_attachments(&multiple, |_| Ok(vec![source.clone()]));
    assert!(matches!(
        result,
        Err(StoreError::Conflict(
            StoreConflict::ChatAttachmentLimitExceeded(_)
        ))
    ));
    source.message.text.push('x');
    let result = materialize_attachments(&single, |_| Ok(vec![source.clone()]));
    assert!(matches!(
        result,
        Err(StoreError::Conflict(
            StoreConflict::ChatAttachmentLimitExceeded(_)
        ))
    ));
    Ok(())
}
