use ahp_server_core::{
    ActionReplay, ChangeSession, CreateSession, FinishTurn, HostStore, MarkSessionReady,
    MemoryHostStore, ModelCatalog, SessionMutation, SnapshotRequest, TurnStart,
};
use ahp_types::actions::{ActionOrigin, ChatTurnStartedAction, StateAction};
use ahp_types::state::{SessionActiveClient, SessionState, SessionStatus, SnapshotState};
use serde_json::json;

fn ready() -> Result<MemoryHostStore, Box<dyn std::error::Error>> {
    let store = MemoryHostStore::new();
    store.create_session(CreateSession {
        session: "ahp-session:/state".into(),
        default_chat: "ahp-chat:/state".into(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: ModelCatalog::single("test-model").default_selection,
    })?;
    store.mark_session_ready(MarkSessionReady {
        session: "ahp-session:/state".into(),
    })?;
    Ok(store)
}

fn state(store: &dyn HostStore) -> Result<SessionState, Box<dyn std::error::Error>> {
    let snapshot = store.snapshot(SnapshotRequest {
        channel: "ahp-session:/state".into(),
        model_catalog: ModelCatalog::single("test-model"),
        server_tools: Vec::new(),
    })?;
    match snapshot.state {
        SnapshotState::Session(state) => Ok(*state),
        _ => Err("expected session snapshot".into()),
    }
}

fn change(
    store: &dyn HostStore,
    mutation: SessionMutation,
) -> Result<(), Box<dyn std::error::Error>> {
    let action = mutation.action();
    let origin = ActionOrigin {
        client_id: "writer".into(),
        client_seq: 7,
    };
    let before = store.server_sequence()?;
    let committed = store.change_session(ChangeSession {
        session: "ahp-session:/state".into(),
        mutation,
        origin: Some(origin.clone()),
    })?;
    assert_eq!(committed.envelopes.len(), 1);
    assert_eq!(committed.envelopes[0].action, action);
    assert_eq!(committed.envelopes[0].origin, Some(origin));
    match store.replay_since(before)? {
        ActionReplay::Replay(actions) => assert_eq!(actions, committed.envelopes),
        ActionReplay::SnapshotRequired => return Err("fresh action must be replayable".into()),
    }
    Ok(())
}

#[test]
fn session_flags_are_independent_and_match_all_summary_projections(
) -> Result<(), Box<dyn std::error::Error>> {
    let store = ready()?;
    change(&store, SessionMutation::Title("Renamed session".into()))?;
    change(&store, SessionMutation::IsArchived(true))?;
    change(&store, SessionMutation::IsRead(true))?;
    let flags = SessionStatus::IsRead.bits() | SessionStatus::IsArchived.bits();
    assert_eq!(state(&store)?.status & flags, flags);
    change(&store, SessionMutation::IsRead(false))?;
    assert_eq!(
        state(&store)?.status & flags,
        SessionStatus::IsArchived.bits()
    );
    change(&store, SessionMutation::IsRead(true))?;
    change(&store, SessionMutation::IsArchived(false))?;
    let snapshot = state(&store)?;
    assert_eq!(snapshot.status & flags, SessionStatus::IsRead.bits());
    assert_eq!(snapshot.title, "Renamed session");
    let summary = store.session_summary("ahp-session:/state")?;
    assert_eq!(summary.title, snapshot.title);
    assert_eq!(summary.status, snapshot.status);
    assert_eq!(store.list_sessions()?[0], summary);
    let before = store.server_sequence()?;
    assert!(store
        .change_session(ChangeSession {
            session: "ahp-session:/missing".into(),
            mutation: SessionMutation::IsRead(false),
            origin: None,
        })
        .is_err());
    assert_eq!(store.server_sequence()?, before);
    assert_eq!(state(&store)?.status, snapshot.status);
    Ok(())
}

#[test]
fn active_clients_upsert_by_id_and_removal_is_idempotent() -> Result<(), Box<dyn std::error::Error>>
{
    let store = ready()?;
    let mut first: SessionActiveClient =
        serde_json::from_value(json!({"clientId":"first", "tools":[]}))?;
    let second = serde_json::from_value(json!({"clientId":"second", "tools":[]}))?;
    change(&store, SessionMutation::ActiveClientSet(first.clone()))?;
    change(&store, SessionMutation::ActiveClientSet(second))?;
    first.display_name = Some("Updated".into());
    change(&store, SessionMutation::ActiveClientSet(first.clone()))?;
    let clients = state(&store)?.active_clients;
    assert_eq!(clients.len(), 2);
    assert_eq!(clients[0], first);
    change(&store, SessionMutation::ActiveClientRemoved("first".into()))?;
    change(&store, SessionMutation::ActiveClientRemoved("first".into()))?;
    assert_eq!(state(&store)?.active_clients[0].client_id, "second");
    Ok(())
}

#[test]
fn turn_lifecycle_keeps_optional_activity_absent_and_preserves_session_flags(
) -> Result<(), Box<dyn std::error::Error>> {
    let store = ready()?;
    change(&store, SessionMutation::IsArchived(true))?;
    change(&store, SessionMutation::IsRead(true))?;
    let before = store.server_sequence()?;
    let action: ChatTurnStartedAction = serde_json::from_value(json!({
        "turnId": "turn", "startedAt": "2026-10-24T00:00:00.000Z",
        "message": {"text": "hello", "origin": {"kind": "user"}}
    }))?;
    store.start_turn(TurnStart {
        chat: "ahp-chat:/state".into(),
        action,
        origin: None,
        legacy_default_selection: None,
    })?;
    assert_eq!(state(&store)?.chats[0].activity, None);
    store.finish_turn(FinishTurn::Complete {
        chat: "ahp-chat:/state".into(),
        turn_id: "turn".into(),
        model_cursor: None,
    })?;
    let snapshot = state(&store)?;
    assert_eq!(snapshot.activity, None);
    assert_eq!(snapshot.chats[0].activity, None);
    let flags = SessionStatus::IsRead.bits() | SessionStatus::IsArchived.bits();
    assert_eq!(snapshot.status & flags, flags);
    let ActionReplay::Replay(actions) = store.replay_since(before)? else {
        return Err("fresh actions must be replayable".into());
    };
    for envelope in actions {
        if let StateAction::SessionChatUpdated(action) = envelope.action {
            assert_eq!(action.changes.activity, None);
        }
    }
    Ok(())
}
