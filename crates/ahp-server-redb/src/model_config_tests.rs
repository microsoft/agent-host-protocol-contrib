use std::collections::HashMap;
use std::path::PathBuf;

use ahp_server_core::{
    ActionReplay, ChangeDraft, CreateSession, FinishTurn, HostStore, MarkSessionReady,
    ModelCatalog, SnapshotRequest, StoreConflict, StoreError, TurnStart, UpdateSessionConfig,
};
use ahp_types::actions::{ActionOrigin, ChatTurnStartedAction, StateAction};
use ahp_types::state::{Message, ModelSelection, SnapshotState};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{RedbHostStore, RedbStoreError};

mod chat;
mod draft;
mod history;
mod stability;

struct Fixture {
    path: PathBuf,
    store: RedbHostStore,
}

impl Fixture {
    fn new() -> Result<Self, RedbStoreError> {
        let path = std::env::temp_dir().join(format!("ahp-model-config-{}.redb", Uuid::new_v4()));
        Ok(Self {
            store: RedbHostStore::open(&path)?,
            path,
        })
    }

    fn reopen(&mut self) -> Result<(), RedbStoreError> {
        self.store.close()?;
        self.store = RedbHostStore::open(&self.path)?;
        Ok(())
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

fn catalog() -> ModelCatalog {
    let alpha = ModelCatalog::single("alpha").models.remove(0);
    let beta = ModelCatalog::single("beta").models.remove(0);
    ModelCatalog {
        models: vec![alpha, beta],
        default_selection: Some(ModelSelection {
            id: "alpha".into(),
            config: None,
        }),
    }
}

fn selection(id: &str, thinking_level: &str) -> ModelSelection {
    ModelSelection {
        id: id.into(),
        config: Some(HashMap::from([(
            "thinkingLevel".into(),
            Value::String(thinking_level.into()),
        )])),
    }
}

fn start(id: &str) -> Result<ChatTurnStartedAction, serde_json::Error> {
    Ok(ChatTurnStartedAction {
        turn_id: id.into(),
        started_at: "2026-09-01T00:00:00.000Z".into(),
        message: serde_json::from_value::<Message>(json!({
            "text": "test", "origin": { "kind": "user" }
        }))?,
        queued_message_id: None,
        meta: None,
    })
}

#[test]
fn model_config_actions_and_effective_models_survive_restart(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    let session = "ahp-session:/model-config";
    let chat = "ahp-chat:/model-config";
    fixture.store.create_session(CreateSession {
        session: session.into(),
        default_chat: chat.into(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: Some(selection("beta", "medium")),
    })?;
    fixture.store.mark_session_ready(MarkSessionReady {
        session: session.into(),
    })?;
    let first = fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action: start("first")?,
        origin: None,
        legacy_default_selection: catalog().default_selection,
    })?;
    assert_eq!(first.value.model_selection, selection("beta", "medium"));
    fixture.store.finish_turn(FinishTurn::Complete {
        chat: chat.into(),
        turn_id: "first".into(),
        model_cursor: None,
    })?;
    let change = fixture.store.update_session_config(UpdateSessionConfig {
        session: session.into(),
        model_selection: selection("alpha", "high"),
        origin: Some(ActionOrigin {
            client_id: "client".into(),
            client_seq: 9,
        }),
    })?;
    assert_eq!(change.envelopes.len(), 1);
    assert_eq!(
        change.envelopes[0].action,
        StateAction::SessionConfigChanged(ahp_types::actions::SessionConfigChangedAction {
            config: ModelCatalog::session_config_values(&selection("alpha", "high")),
            replace: Some(true),
        })
    );
    fixture.reopen()?;
    let ActionReplay::Replay(replay) = fixture.store.replay_since(0)? else {
        return Err("expected replayable actions".into());
    };
    assert!(replay.iter().any(|envelope| {
        matches!(envelope.action, StateAction::SessionConfigChanged(_))
            && envelope
                .origin
                .as_ref()
                .is_some_and(|origin| origin.client_seq == 9)
    }));
    let state = fixture.store.snapshot(SnapshotRequest {
        channel: session.into(),
        model_catalog: catalog(),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Session(state) = state.state else {
        return Err("expected session snapshot".into());
    };
    assert_eq!(
        state.config.as_ref().map(|config| &config.values),
        Some(&ModelCatalog::session_config_values(&selection(
            "alpha", "high"
        )))
    );

    let second = fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action: start("second")?,
        origin: None,
        legacy_default_selection: catalog().default_selection,
    })?;
    assert_eq!(second.value.model_selection, selection("alpha", "high"));
    fixture.store.finish_turn(FinishTurn::Complete {
        chat: chat.into(),
        turn_id: "second".into(),
        model_cursor: None,
    })?;
    let chat_state = fixture.store.snapshot(SnapshotRequest {
        channel: chat.into(),
        model_catalog: catalog(),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Chat(chat_state) = chat_state.state else {
        return Err("expected chat snapshot".into());
    };
    assert_eq!(
        chat_state.turns[0].message.model,
        Some(selection("beta", "medium"))
    );
    assert_eq!(
        chat_state.turns[1].message.model,
        Some(selection("alpha", "high"))
    );
    Ok(())
}

#[test]
fn legacy_session_uses_the_catalog_default_when_projected_and_started(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let session = "ahp-session:/legacy-model";
    let chat = "ahp-chat:/legacy-model";
    fixture.store.create_session(CreateSession {
        session: session.into(),
        default_chat: chat.into(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: Some(selection("beta", "medium")),
    })?;
    fixture.store.test_write(|write| {
        let mut session = crate::sessions::load_session(write, session)?
            .ok_or_else(|| ahp_server_core::StoreError::Backend("missing session".into()))?;
        session.model_selection = None;
        crate::sessions::write_session(write, &session)
    })?;
    let snapshot = fixture.store.snapshot(SnapshotRequest {
        channel: session.into(),
        model_catalog: catalog(),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Session(state) = snapshot.state else {
        return Err("expected session snapshot".into());
    };
    assert_eq!(
        state
            .config
            .as_ref()
            .and_then(|config| config.values.get("model")),
        Some(&json!({ "id": "alpha" }))
    );
    let started = fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action: start("legacy-turn")?,
        origin: None,
        legacy_default_selection: catalog().default_selection,
    })?;
    assert_eq!(started.value.model_selection.id, "alpha");
    Ok(())
}

#[test]
fn legacy_session_without_a_catalog_default_requires_repair_before_execution(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    let session = "ahp-session:/legacy-no-default";
    let chat = "ahp-chat:/legacy-no-default";
    fixture.store.create_session(CreateSession {
        session: session.into(),
        default_chat: chat.into(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: Some(selection("alpha", "medium")),
    })?;
    fixture.store.test_write(|write| {
        let mut session = crate::sessions::load_session(write, session)?
            .ok_or_else(|| StoreError::Backend("missing session".into()))?;
        session.model_selection = None;
        crate::sessions::write_session(write, &session)
    })?;
    fixture.reopen()?;

    let catalog = ModelCatalog::new([("alpha", "Alpha")]);
    let snapshot = fixture.store.snapshot(SnapshotRequest {
        channel: session.into(),
        model_catalog: catalog,
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Session(state) = snapshot.state else {
        return Err("expected session snapshot".into());
    };
    let config = state.config.ok_or("missing session config")?;
    assert!(config.values.get("model").is_none());
    assert_eq!(config.schema.required, Some(vec!["model".into()]));

    assert!(matches!(
        fixture.store.start_turn(TurnStart {
            chat: chat.into(),
            action: start("unrepaired")?,
            origin: None,
            legacy_default_selection: None,
        }),
        Err(StoreError::Conflict(StoreConflict::MissingModelSelection(
            _
        )))
    ));
    let explicit = serde_json::from_value(json!({
        "turnId": "explicit-model",
        "startedAt": "2026-09-02T00:00:00.000Z",
        "message": {
            "text": "explicit selection",
            "origin": { "kind": "user" },
            "model": { "id": "alpha", "config": { "thinkingLevel": "medium" } }
        }
    }))?;
    let started = fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action: explicit,
        origin: None,
        legacy_default_selection: None,
    })?;
    assert_eq!(started.value.model_selection, selection("alpha", "medium"));
    fixture.store.finish_turn(FinishTurn::Complete {
        chat: chat.into(),
        turn_id: "explicit-model".into(),
        model_cursor: None,
    })?;
    let snapshot = fixture.store.snapshot(SnapshotRequest {
        channel: chat.into(),
        model_catalog: ModelCatalog::new([("alpha", "Alpha")]),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Chat(snapshot) = snapshot.state else {
        return Err("expected chat snapshot".into());
    };
    assert_eq!(
        snapshot.turns[0].message.model,
        Some(selection("alpha", "medium"))
    );
    fixture.store.change_draft(ChangeDraft {
        chat: chat.into(),
        draft: Some(serde_json::from_value(json!({
            "text": "draft selection", "origin": { "kind": "user" },
            "model": { "id": "alpha", "config": { "thinkingLevel": "medium" } }
        }))?),
        turn_precondition: Default::default(),
        origin: None,
    })?;
    let started = fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action: start("draft-model")?,
        origin: None,
        legacy_default_selection: None,
    })?;
    assert_eq!(started.value.model_selection, selection("alpha", "medium"));
    fixture.store.finish_turn(FinishTurn::Complete {
        chat: chat.into(),
        turn_id: "draft-model".into(),
        model_cursor: None,
    })?;
    fixture.store.update_session_config(UpdateSessionConfig {
        session: session.into(),
        model_selection: selection("alpha", "medium"),
        origin: None,
    })?;
    let started = fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action: start("repaired")?,
        origin: None,
        legacy_default_selection: None,
    })?;
    assert_eq!(started.value.model_selection, selection("alpha", "medium"));
    Ok(())
}
