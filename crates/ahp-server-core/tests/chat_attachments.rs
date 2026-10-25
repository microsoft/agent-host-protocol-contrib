use ahp_server_core::{
    ChangeChat, ChatMutation, CreateSession, FinishTurn, HostStore, MarkSessionReady,
    MemoryHostStore, ModelCatalog, TurnStart,
};
use ahp_types::actions::ChatTurnStartedAction;
use serde_json::{json, Value};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn ready(store: &dyn HostStore, id: &str) -> Result {
    store.create_session(CreateSession {
        session: format!("ahp-session:/{id}"),
        default_chat: format!("ahp-chat:/{id}"),
        provider: "test".into(),
        workspace: "file:///workspace".into(),
        model_selection: ModelCatalog::single("test").default_selection,
    })?;
    store.mark_session_ready(MarkSessionReady {
        session: format!("ahp-session:/{id}"),
    })?;
    Ok(())
}

fn action(id: &str, attachments: Value) -> Result<ChatTurnStartedAction> {
    Ok(serde_json::from_value(json!({
        "turnId":id, "startedAt":"2026-10-24T00:00:00.000Z",
        "message":{"text":format!("Remember this phrase: \"{id}\""), "origin":{"kind":"user"}, "attachments":attachments}
    }))?)
}

fn start(
    store: &dyn HostStore,
    chat: &str,
    id: &str,
    attachments: Value,
) -> Result<ahp_server_core::StartedTurn> {
    Ok(store
        .start_turn(TurnStart {
            chat: format!("ahp-chat:/{chat}"),
            action: action(id, attachments)?,
            origin: None,
            legacy_default_selection: None,
        })?
        .value)
}

fn finish(store: &dyn HostStore, chat: &str, id: &str) -> Result {
    store.finish_turn(FinishTurn::Complete {
        chat: format!("ahp-chat:/{chat}"),
        turn_id: id.into(),
        model_cursor: None,
    })?;
    Ok(())
}

fn attachment(chat: &str, end: Option<&str>) -> Value {
    let mut attachment =
        json!({"type":"chat", "resource":format!("ahp-chat:/{chat}"), "label":"Source"});
    if let Some(end) = end {
        attachment["endTurn"] = json!(end);
    }
    json!([attachment])
}

#[test]
fn invalid_bounds_are_atomic_and_empty_transcripts_are_allowed() -> Result {
    let store = MemoryHostStore::new();
    ready(&store, "source")?;
    ready(&store, "target")?;
    for attachments in [
        attachment("missing", None),
        attachment("source", Some("unknown")),
    ] {
        let sequence = store.server_sequence()?;
        assert!(start(&store, "target", "invalid", attachments).is_err());
        assert_eq!(store.server_sequence()?, sequence);
    }
    assert!(
        start(&store, "target", "empty", attachment("source", None))?.materialized_attachments
            ["empty"]
            .is_empty()
    );
    finish(&store, "target", "empty")?;
    start(&store, "source", "active", json!([]))?;
    let sequence = store.server_sequence()?;
    assert!(start(
        &store,
        "target",
        "invalid",
        attachment("source", Some("active"))
    )
    .is_err());
    assert_eq!(store.server_sequence()?, sequence);
    finish(&store, "source", "active")?;
    store.change_chat(ChangeChat {
        chat: "ahp-chat:/source".into(),
        mutation: ChatMutation::Truncated(serde_json::from_value(json!({}))?),
        origin: None,
    })?;
    let sequence = store.server_sequence()?;
    assert!(start(
        &store,
        "target",
        "invalid",
        attachment("source", Some("active"))
    )
    .is_err());
    assert_eq!(store.server_sequence()?, sequence);
    Ok(())
}

#[test]
fn explicit_and_implicit_bounds_stay_pinned_after_source_pruning() -> Result {
    let store = MemoryHostStore::new();
    for id in ["source", "explicit", "implicit", "nested"] {
        ready(&store, id)?;
    }
    start(&store, "nested", "nested-memory", json!([]))?;
    finish(&store, "nested", "nested-memory")?;
    start(&store, "source", "first", attachment("nested", None))?;
    finish(&store, "source", "first")?;
    start(&store, "source", "second", json!([]))?;
    finish(&store, "source", "second")?;
    let explicit = start(
        &store,
        "explicit",
        "attached",
        attachment("source", Some("first")),
    )?;
    assert_eq!(explicit.materialized_attachments["attached"].len(), 1);
    assert_eq!(explicit.materialized_attachments["attached"][0].id, "first");
    assert!(!explicit.materialized_attachments["attached"][0]
        .message
        .attachments
        .as_ref()
        .expect("nested reference")
        .is_empty());
    finish(&store, "explicit", "attached")?;
    let implicit = start(&store, "implicit", "attached", attachment("source", None))?;
    assert_eq!(implicit.materialized_attachments["attached"].len(), 2);
    finish(&store, "implicit", "attached")?;
    for id in ["source", "nested"] {
        store.change_chat(ChangeChat {
            chat: format!("ahp-chat:/{id}"),
            mutation: ChatMutation::Truncated(serde_json::from_value(json!({}))?),
            origin: None,
        })?;
    }
    let next = start(&store, "explicit", "next", json!([]))?;
    assert_eq!(
        next.materialized_attachments["attached"],
        explicit.materialized_attachments["attached"]
    );
    let next = start(&store, "implicit", "next", json!([]))?;
    assert_eq!(
        next.materialized_attachments["attached"],
        implicit.materialized_attachments["attached"]
    );
    Ok(())
}

#[test]
fn queued_messages_pin_attachments_when_enqueued_not_when_started() -> Result {
    let store = MemoryHostStore::new();
    ready(&store, "source")?;
    ready(&store, "target")?;
    start(&store, "source", "first", json!([]))?;
    finish(&store, "source", "first")?;
    let queued = action("queued", attachment("source", None))?;
    store.change_chat(ChangeChat {
        chat: "ahp-chat:/target".into(),
        mutation: ChatMutation::PendingSet(serde_json::from_value(json!({
            "id":"queue", "kind":"queued", "message":queued.message
        }))?),
        origin: None,
    })?;
    start(&store, "source", "second", json!([]))?;
    finish(&store, "source", "second")?;
    let mut queued = queued;
    queued.queued_message_id = Some("queue".into());
    let started = store
        .start_turn(TurnStart {
            chat: "ahp-chat:/target".into(),
            action: queued,
            origin: None,
            legacy_default_selection: None,
        })?
        .value;
    assert_eq!(started.materialized_attachments["queued"].len(), 1);
    assert_eq!(started.materialized_attachments["queued"][0].id, "first");
    Ok(())
}

#[test]
fn attachment_limits_reject_turn_and_pending_acceptance_atomically() -> Result {
    let store = MemoryHostStore::new();
    ready(&store, "source")?;
    ready(&store, "target")?;
    let attachments = json!(vec![
        json!({"type":"chat", "resource":"ahp-chat:/source", "label":"Source"});
        17
    ]);
    let sequence = store.server_sequence()?;
    let result = start(&store, "target", "over-limit", attachments.clone());
    let error = result
        .err()
        .ok_or("attachment count limit must reject the turn")?;
    assert!(error.to_string().contains("chat attachment limit exceeded"));
    assert_eq!(store.server_sequence()?, sequence);
    let result = store.change_chat(ChangeChat {
        chat: "ahp-chat:/target".into(),
        mutation: ChatMutation::PendingSet(serde_json::from_value(json!({
            "id":"over-limit", "kind":"queued", "message":action("queued", attachments.clone())?.message
        }))?),
        origin: None,
    });
    assert!(matches!(
        result,
        Err(ahp_server_core::StoreError::Conflict(
            ahp_server_core::StoreConflict::ChatAttachmentLimitExceeded(_)
        ))
    ));
    assert_eq!(store.server_sequence()?, sequence);
    assert!(store
        .pending_messages("ahp-chat:/target")?
        .entries
        .is_empty());
    store.change_chat(ChangeChat {
        chat: "ahp-chat:/target".into(),
        mutation: ChatMutation::PendingSet(serde_json::from_value(json!({
            "id":"accepted", "kind":"queued", "message":action("queued", json!([]))?.message
        }))?),
        origin: None,
    })?;
    let sequence = store.server_sequence()?;
    let mut changed = action("changed-after-queue", attachments)?;
    changed.queued_message_id = Some("accepted".into());
    let result = store.start_turn(TurnStart {
        chat: "ahp-chat:/target".into(),
        action: changed,
        origin: None,
        legacy_default_selection: None,
    });
    assert!(matches!(
        result,
        Err(ahp_server_core::StoreError::Conflict(
            ahp_server_core::StoreConflict::ChatAttachmentLimitExceeded(_)
        ))
    ));
    assert_eq!(store.server_sequence()?, sequence);
    start(&store, "target", "valid-after-rejection", json!([]))?;
    Ok(())
}
