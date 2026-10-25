use ahp_server_core::{
    ChangeChat, ChatMutation, CreateSession, FinishTurn, HostStore, ModelCursor, TurnStart,
};
use ahp_types::actions::{
    ChatPendingMessageSetAction, ChatQueuedMessagesReorderedAction, ChatTruncatedAction,
    StateAction,
};
use ahp_types::state::PendingMessageKind;

use super::{catalog, start, Fixture};

fn create(fixture: &Fixture) -> Result<(), ahp_server_core::StoreError> {
    fixture.store.create_session(CreateSession {
        session: "ahp-session:/pending-redb".into(),
        default_chat: "ahp-chat:/pending-redb".into(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: catalog().default_selection,
    })?;
    Ok(())
}

#[test]
fn queued_order_and_separate_steering_survive_restart() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    create(&fixture)?;
    let chat = "ahp-chat:/pending-redb";
    for (id, kind) in [
        ("steer", PendingMessageKind::Steering),
        ("queued", PendingMessageKind::Queued),
        ("next", PendingMessageKind::Queued),
    ] {
        fixture.store.change_chat(ChangeChat {
            chat: chat.into(),
            origin: None,
            mutation: ChatMutation::PendingSet(ChatPendingMessageSetAction {
                id: id.into(),
                kind,
                message: start(id)?.message,
            }),
        })?;
    }
    fixture.store.change_chat(ChangeChat {
        chat: chat.into(),
        origin: None,
        mutation: ChatMutation::Reordered(ChatQueuedMessagesReorderedAction {
            order: vec!["next".into()],
        }),
    })?;
    fixture.reopen()?;
    let pending = fixture.store.pending_messages(chat)?;
    assert_eq!(
        pending
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["next", "queued"]
    );
    assert!(serde_json::to_value(&pending.entries)?[0]
        .get("kind")
        .is_none());
    assert_eq!(
        pending.steering.as_ref().map(|entry| entry.id.as_str()),
        Some("steer")
    );
    let mut action = start("auto")?;
    action.queued_message_id = Some("next".into());
    let transition = fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action,
        origin: None,
        legacy_default_selection: None,
    })?;
    assert!(
        matches!(&transition.envelopes[0].action, StateAction::ChatPendingMessageRemoved(action) if action.id == "next")
    );
    assert!(
        matches!(&transition.envelopes[1].action, StateAction::ChatTurnStarted(action) if action.queued_message_id.as_deref() == Some("next"))
    );
    fixture.store.finish_turn(FinishTurn::Complete {
        chat: chat.into(),
        turn_id: "auto".into(),
        model_cursor: None,
    })?;
    fixture.reopen()?;
    let pending = fixture.store.pending_messages(chat)?;
    assert_eq!(pending.entries.len(), 1);
    assert_eq!(pending.entries[0].id, "queued");
    assert_eq!(
        pending.steering.as_ref().map(|entry| entry.id.as_str()),
        Some("steer")
    );
    fixture.store.change_chat(ChangeChat {
        chat: chat.into(),
        origin: None,
        mutation: ChatMutation::PendingSet(ChatPendingMessageSetAction {
            id: "next".into(),
            kind: PendingMessageKind::Queued,
            message: start("auto")?.message,
        }),
    })?;
    assert_eq!(fixture.store.pending_messages(chat)?.entries.len(), 2);
    Ok(())
}

#[test]
fn truncation_clears_provider_cursor_and_terminal_timestamps_match_actions(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    create(&fixture)?;
    let chat = "ahp-chat:/pending-redb";
    for id in ["first", "second"] {
        let action = start(id)?;
        let started_at = action.started_at.clone();
        fixture.store.start_turn(TurnStart {
            chat: chat.into(),
            action,
            origin: None,
            legacy_default_selection: None,
        })?;
        let finish = fixture.store.finish_turn(FinishTurn::Complete {
            chat: chat.into(),
            turn_id: id.into(),
            model_cursor: Some(ModelCursor {
                scope: "scope".into(),
                response_id: id.into(),
                turn_id: id.into(),
            }),
        })?;
        let expected = chrono::DateTime::parse_from_rfc3339(&started_at)?
            + chrono::TimeDelta::milliseconds(finish.value);
        let persisted = fixture.store.get_chat(chat)?.ok_or("chat missing")?;
        assert_eq!(
            persisted.modified_at,
            expected.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        );
    }
    fixture.store.change_chat(ChangeChat {
        chat: chat.into(),
        origin: None,
        mutation: ChatMutation::Truncated(ChatTruncatedAction {
            turn_id: Some("first".into()),
        }),
    })?;
    fixture.reopen()?;
    let persisted = fixture.store.get_chat(chat)?.ok_or("chat missing")?;
    assert_eq!(persisted.turns.len(), 1);
    assert_eq!(persisted.turns[0].id, "first");
    assert!(persisted.model_cursor.is_none());
    let next = fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action: start("third")?,
        origin: None,
        legacy_default_selection: None,
    })?;
    assert_eq!(next.value.history.len(), 1);
    assert_eq!(next.value.history[0].id, "first");
    assert!(next.value.model_cursor.is_none());
    Ok(())
}
