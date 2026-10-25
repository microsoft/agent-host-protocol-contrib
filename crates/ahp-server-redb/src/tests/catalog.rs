use super::*;
use ahp_server_core::{ChangeChat, ChatMutation};

fn assert_projection(
    store: &RedbHostStore,
    session: &str,
    chat: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let owner = session_state(store, session.into())?;
    let summary = owner
        .chats
        .iter()
        .find(|entry| entry.resource == chat)
        .ok_or("catalog entry")?;
    let summary_json = serde_json::to_value(summary)?;
    let chat_json = serde_json::to_value(chat_state(store, chat.into())?)?;
    for (key, value) in summary_json.as_object().ok_or("summary object")? {
        assert_eq!(chat_json.get(key), Some(value), "summary field {key}");
    }
    for key in [
        "activity",
        "changes",
        "origin",
        "movable",
        "interactivity",
        "workingDirectories",
    ] {
        assert_eq!(
            summary_json.get(key),
            chat_json.get(key),
            "optional summary field {key}"
        );
    }
    let root = store.session_summary(session)?;
    assert_eq!(root.default_chat, owner.default_chat);
    let compact = root.chats.ok_or("compact catalog")?;
    assert_eq!(compact.len(), owner.chats.len());
    for (compact, full) in compact.iter().zip(&owner.chats) {
        assert_eq!(compact.resource, full.resource);
        assert_eq!(compact.status, Some(full.status));
    }
    Ok(())
}

#[test]
fn chat_read_flags_and_complete_catalog_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = TestStore::new()?;
    let (session, chat) = create_ready(fixture.store())?;
    fixture.store().create_chat(CreateChat {
        session: session.clone(),
        chat: "ahp-chat:/sibling".into(),
    })?;
    let owner_status = session_state(fixture.store(), session.clone())?.status;
    let before = fixture.store().server_sequence()?;
    let changed = fixture.store().change_chat(ChangeChat {
        chat: chat.clone(),
        mutation: ChatMutation::IsRead(true),
        origin: None,
    })?;
    fixture.reopen()?;
    let store = fixture.store();
    assert_eq!(
        chat_state(store, chat.clone())?.status & SessionStatus::IsRead.bits(),
        SessionStatus::IsRead.bits()
    );
    assert_eq!(
        chat_state(store, "ahp-chat:/sibling".into())?.status & SessionStatus::IsRead.bits(),
        0
    );
    assert_eq!(session_state(store, session.clone())?.status, owner_status);
    assert_projection(store, &session, &chat)?;
    assert_eq!(
        store.replay_since(before)?,
        ActionReplay::Replay(changed.envelopes)
    );
    store.start_turn(TurnStart {
        chat: chat.clone(),
        action: start("turn")?,
        origin: None,
        legacy_default_selection: None,
    })?;
    assert_eq!(
        chat_state(store, chat.clone())?.status & SessionStatus::IsRead.bits(),
        0
    );
    assert_projection(store, &session, &chat)?;
    store.finish_turn(FinishTurn::Complete {
        chat: chat.clone(),
        turn_id: "turn".into(),
        model_cursor: None,
    })?;
    assert_projection(store, &session, &chat)?;
    Ok(())
}

#[test]
fn confirmation_aggregate_and_recovery_removal_share_the_state_journal(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = TestStore::new()?;
    let (session, chat) = create_ready(fixture.store())?;
    let store = fixture.store();
    store.start_turn(TurnStart {
        chat: chat.clone(),
        action: start("turn")?,
        origin: None,
        legacy_default_selection: None,
    })?;
    store.start_tool(
        &chat,
        "turn",
        streaming_part("tool"),
        tool_start_action("turn", "tool"),
    )?;
    let pending = pending_confirmation_state("tool")?;
    let changed = store.transition_tool(ToolCallTransition {
        chat: chat.clone(), turn_id: "turn".into(), tool_call_id: "tool".into(), next_state: pending.clone(),
        actions: vec![serde_json::from_value(json!({"type":"chat/toolCallReady", "turnId":"turn", "toolCallId":"tool", "invocationMessage":"Replace text", "toolInput":"{}"}))?], origin: None, allow_command: None,
    })?;
    let aggregate = session_state(store, session.clone())?
        .input_needed
        .ok_or("aggregate")?;
    assert_eq!(
        serde_json::to_value(&aggregate[0])?["toolCall"],
        serde_json::to_value(pending)?
    );
    assert!(changed.envelopes.iter().any(|event| matches!(&event.action, StateAction::SessionInputNeededSet(action) if action.request == aggregate[0])));
    assert_projection(store, &session, &chat)?;
    let before = store.server_sequence()?;
    fixture.reopen()?;
    assert!(session_state(fixture.store(), session.clone())?
        .input_needed
        .is_none());
    let ActionReplay::Replay(actions) = fixture.store().replay_since(before)? else {
        return Err("recovery replay".into());
    };
    let request = serde_json::to_value(&aggregate[0])?;
    assert!(actions.iter().any(|event| matches!(&event.action, StateAction::SessionInputNeededRemoved(action) if serde_json::Value::String(action.id.clone()) == request["id"])));
    assert_projection(fixture.store(), &session, &chat)?;
    Ok(())
}

#[test]
fn reconciliation_reads_only_affected_sessions_and_skips_text_delta_catalogs(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = TestStore::new()?;
    let store = fixture.store();
    let (session, chat) = create_ready(store)?;
    let (_, unrelated_chat) = create_ready(store)?;
    let sibling = "ahp-chat:/stream-sibling";
    store.create_chat(CreateChat {
        session,
        chat: sibling.into(),
    })?;
    store.start_turn(TurnStart {
        chat: chat.clone(),
        action: start("turn")?,
        origin: None,
        legacy_default_selection: None,
    })?;
    store.test_write(|write| {
        let mut table = write.open_table(CHATS).map_err(crate::db::backend)?;
        table
            .insert(unrelated_chat.as_str(), "invalid JSON")
            .map_err(crate::db::backend)?;
        Ok(())
    })?;
    store.change_chat(ChangeChat {
        chat: chat.clone(),
        mutation: ChatMutation::IsRead(true),
        origin: None,
    })?;
    store.test_write(|write| {
        let mut table = write.open_table(CHATS).map_err(crate::db::backend)?;
        table
            .insert(sibling, "invalid JSON")
            .map_err(crate::db::backend)?;
        Ok(())
    })?;
    let before = store.server_sequence()?;
    let appended = store.append_text(
        &chat,
        "turn",
        &crate::types::response_part_id("turn"),
        "chunk",
    )?;
    assert_eq!(appended.envelopes.len(), 1);
    assert!(
        matches!(&appended.envelopes[0].action, StateAction::ChatDelta(action) if action.content == "chunk")
    );
    assert_eq!(store.server_sequence()?, before + 1);
    Ok(())
}

#[test]
fn committed_before_catalog_is_empty_before_session_and_chat_tables_exist(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = TestStore::new()?;
    let database = fixture.store().database()?;
    let read = database.get()?.begin_read().map_err(crate::db::backend)?;
    let sessions = std::collections::BTreeSet::from(["ahp-session:/first".to_owned()]);
    assert!(crate::catalog::catalogs_before(&read, &sessions)?.is_empty());
    assert!(crate::db::read_record::<crate::types::PersistedChat>(
        &read,
        CHATS,
        "ahp-chat:/first",
    )?
    .is_none());
    Ok(())
}
