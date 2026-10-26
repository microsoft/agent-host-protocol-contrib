use std::path::PathBuf;

use ahp_server_core::{
    ActionReplay, ChangeSession, CreateSession, HostStore, MarkSessionReady, ModelCatalog,
    SessionMutation, SnapshotRequest,
};
use ahp_types::actions::ActionOrigin;
use ahp_types::state::{SessionState, SessionStatus, SnapshotState};
use serde_json::json;
use uuid::Uuid;

use crate::RedbHostStore;

struct Fixture {
    path: PathBuf,
    store: RedbHostStore,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!("ahp-session-state-{}.redb", Uuid::new_v4()));
        let store = RedbHostStore::open(&path)?;
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
        Ok(Self { path, store })
    }

    fn reopen(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.store.close()?;
        self.store = RedbHostStore::open(&self.path)?;
        Ok(())
    }

    fn state(&self) -> Result<SessionState, Box<dyn std::error::Error>> {
        let snapshot = self.store.snapshot(SnapshotRequest {
            channel: "ahp-session:/state".into(),
            model_catalog: ModelCatalog::single("test-model"),
            server_tools: Vec::new(),
        })?;
        match snapshot.state {
            SnapshotState::Session(state) => Ok(*state),
            _ => Err("expected session snapshot".into()),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.store.close();
        for suffix in ["", ".lock"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

#[test]
fn session_metadata_and_journal_survive_reopen_together() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = Fixture::new()?;
    let origin = ActionOrigin {
        client_id: "writer".into(),
        client_seq: 7,
    };
    let before = fixture.store.server_sequence()?;
    let mut committed = Vec::new();
    for mutation in [
        SessionMutation::Title("Durable title".into()),
        SessionMutation::IsArchived(true),
        SessionMutation::IsRead(true),
        SessionMutation::IsRead(false),
        SessionMutation::IsRead(true),
        SessionMutation::IsArchived(false),
    ] {
        let expected = mutation.action();
        let transition = fixture.store.change_session(ChangeSession {
            session: "ahp-session:/state".into(),
            mutation,
            origin: Some(origin.clone()),
        })?;
        assert_eq!(transition.envelopes[0].action, expected);
        assert_eq!(transition.envelopes[0].origin, Some(origin.clone()));
        committed.extend(transition.envelopes);
    }
    fixture.reopen()?;
    let state = fixture.state()?;
    assert_eq!(state.title, "Durable title");
    let flags = SessionStatus::IsRead.bits() | SessionStatus::IsArchived.bits();
    assert_eq!(state.status & flags, SessionStatus::IsRead.bits());
    let summary = fixture.store.session_summary("ahp-session:/state")?;
    assert_eq!(summary.title, state.title);
    assert_eq!(summary.status, state.status);
    assert_eq!(fixture.store.list_sessions()?[0], summary);
    match fixture.store.replay_since(before)? {
        ActionReplay::Replay(actions) => assert_eq!(actions, committed),
        ActionReplay::SnapshotRequired => return Err("fresh actions must survive reopen".into()),
    }
    let sequence = fixture.store.server_sequence()?;
    assert!(fixture
        .store
        .change_session(ChangeSession {
            session: "ahp-session:/missing".into(),
            mutation: SessionMutation::IsRead(false),
            origin: None,
        })
        .is_err());
    assert_eq!(fixture.store.server_sequence()?, sequence);
    assert_eq!(fixture.state()?.status, state.status);
    Ok(())
}

#[test]
fn restart_clears_active_clients_with_replayable_idempotent_removals(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    for client in [
        json!({"clientId":"first", "tools":[]}),
        json!({"clientId":"second", "tools":[]}),
        json!({"clientId":"first", "tools":[], "displayName":"Updated"}),
    ] {
        fixture.store.change_session(ChangeSession {
            session: "ahp-session:/state".into(),
            mutation: SessionMutation::ActiveClientSet(serde_json::from_value(client)?),
            origin: None,
        })?;
    }
    let state = fixture.state()?;
    assert_eq!(state.active_clients.len(), 2);
    assert_eq!(state.active_clients[0].client_id, "first");
    assert_eq!(
        state.active_clients[0].display_name.as_deref(),
        Some("Updated")
    );
    for _ in 0..2 {
        fixture.store.change_session(ChangeSession {
            session: "ahp-session:/state".into(),
            mutation: SessionMutation::ActiveClientRemoved("first".into()),
            origin: None,
        })?;
    }
    assert_eq!(fixture.state()?.active_clients.len(), 1);
    let before = fixture.store.server_sequence()?;
    fixture.reopen()?;
    assert!(fixture.state()?.active_clients.is_empty());
    let ActionReplay::Replay(actions) = fixture.store.replay_since(before)? else {
        return Err("recovery removal must be replayable".into());
    };
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].channel, "ahp-session:/state");
    assert_eq!(
        serde_json::to_value(&actions[0].action)?,
        json!({
            "type":"session/activeClientRemoved", "clientId":"second"
        })
    );
    assert!(actions[0].origin.is_none());
    let recovered = fixture.store.server_sequence()?;
    fixture.reopen()?;
    assert!(fixture.state()?.active_clients.is_empty());
    assert_eq!(fixture.store.server_sequence()?, recovered);
    Ok(())
}

#[test]
fn resource_watch_envelopes_share_the_durable_server_sequence(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    let before = fixture.store.server_sequence()?;
    let committed = fixture.store.publish_resource_change(
        "ahp-resource-watch:/watch".into(),
        ahp_types::actions::ResourceWatchChangedAction {
            changes: json!({"items": []}),
        },
    )?;
    assert_eq!(fixture.store.server_sequence()?, before + 1);
    fixture.reopen()?;
    match fixture.store.replay_since(before)? {
        ActionReplay::Replay(actions) => assert_eq!(actions, committed.envelopes),
        ActionReplay::SnapshotRequired => return Err("watch envelope must survive reopen".into()),
    }
    Ok(())
}

#[test]
fn legacy_session_records_default_new_metadata_and_membership_fields(
) -> Result<(), Box<dyn std::error::Error>> {
    let session: crate::types::PersistedSession = serde_json::from_value(json!({
        "uri": "ahp-session:/legacy", "provider": "openai", "title": "Legacy",
        "created_at": "2026-10-24T00:00:00.000Z", "lifecycle": "Ready"
    }))?;
    assert_eq!(session.status_flags, 0);
    assert!(session.active_clients.is_empty());
    Ok(())
}

#[test]
fn completed_turn_has_no_stale_activity_and_preserves_session_flags_after_reopen(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    for mutation in [
        SessionMutation::IsRead(true),
        SessionMutation::IsArchived(true),
    ] {
        fixture.store.change_session(ChangeSession {
            session: "ahp-session:/state".into(),
            mutation,
            origin: None,
        })?;
    }
    let before = fixture.store.server_sequence()?;
    fixture.store.start_turn(ahp_server_core::TurnStart {
        chat: "ahp-chat:/state".into(),
        action: serde_json::from_value(json!({
            "turnId": "turn", "startedAt": "2026-10-24T00:00:00.000Z",
            "message": {"text": "hello", "origin": {"kind": "user"}}
        }))?,
        origin: None,
        legacy_default_selection: None,
    })?;
    assert_eq!(fixture.state()?.chats[0].activity, None);
    fixture
        .store
        .finish_turn(ahp_server_core::FinishTurn::Complete {
            chat: "ahp-chat:/state".into(),
            turn_id: "turn".into(),
            model_cursor: None,
        })?;
    fixture.reopen()?;
    let state = fixture.state()?;
    assert_eq!(state.activity, None);
    assert_eq!(state.chats[0].activity, None);
    let flags = SessionStatus::IsRead.bits() | SessionStatus::IsArchived.bits();
    assert_eq!(state.status & flags, flags);
    let ActionReplay::Replay(actions) = fixture.store.replay_since(before)? else {
        return Err("turn actions must survive reopen".into());
    };
    for envelope in actions {
        if let ahp_types::actions::StateAction::SessionChatUpdated(action) = envelope.action {
            assert_eq!(action.changes.activity, None);
        }
    }
    Ok(())
}

#[test]
fn restart_recovers_interrupted_turns_and_all_active_clients_together(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    for client_id in ["first", "second"] {
        fixture.store.change_session(ChangeSession {
            session: "ahp-session:/state".into(),
            mutation: SessionMutation::ActiveClientSet(serde_json::from_value(json!({
                "clientId":client_id, "tools":[]
            }))?),
            origin: None,
        })?;
    }
    fixture.store.start_turn(ahp_server_core::TurnStart {
        chat: "ahp-chat:/state".into(),
        action: serde_json::from_value(json!({
            "turnId":"interrupted", "startedAt":"2026-10-24T00:00:00.000Z",
            "message":{"text":"hello", "origin":{"kind":"user"}}
        }))?,
        origin: None,
        legacy_default_selection: None,
    })?;
    let before = fixture.store.server_sequence()?;
    fixture.reopen()?;
    let state = fixture.state()?;
    assert!(state.active_clients.is_empty());
    assert!(SessionStatus::from_bits(state.chats[0].status).contains(SessionStatus::Error));
    let ActionReplay::Replay(actions) = fixture.store.replay_since(before)? else {
        return Err("restart recovery must be replayable".into());
    };
    assert_eq!(actions.len(), 4);
    assert!(matches!(
        &actions[0].action,
        ahp_types::actions::StateAction::ChatError(_)
    ));
    assert!(matches!(
        &actions[1].action,
        ahp_types::actions::StateAction::SessionChatUpdated(_)
    ));
    let removed = actions
        .iter()
        .filter_map(|envelope| match &envelope.action {
            ahp_types::actions::StateAction::SessionActiveClientRemoved(action) => {
                Some(action.client_id.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(removed, ["first", "second"]);
    let recovered = fixture.store.server_sequence()?;
    assert_eq!(recovered, before + 4);
    fixture.reopen()?;
    assert_eq!(fixture.store.server_sequence()?, recovered);
    Ok(())
}
