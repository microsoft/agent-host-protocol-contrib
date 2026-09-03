use ahp_server_core::{
    ActionReplay, ChangeDraft, CreateSession, HostStore, MarkSessionReady, SnapshotRequest,
    StoreConflict, StoreError, TurnStart,
};
use ahp_types::actions::{ActionOrigin, ChatDraftChangedAction, StateAction};
use ahp_types::state::{Message, SnapshotState};
use serde_json::json;

use super::{catalog, selection, start, Fixture};

#[test]
fn redb_drafts_survive_restart_and_clear_atomically_when_a_turn_starts(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    let session = "ahp-session:/draft-redb";
    let chat = "ahp-chat:/draft-redb";
    fixture.store.create_session(CreateSession {
        session: session.into(),
        default_chat: chat.into(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: Some(selection("alpha", "medium")),
    })?;
    fixture.store.mark_session_ready(MarkSessionReady {
        session: session.into(),
    })?;
    let draft = message("saved draft", "user")?;
    let change = fixture.store.change_draft(ChangeDraft {
        chat: chat.into(),
        draft: Some(draft.clone()),
        turn_precondition: Default::default(),
        origin: Some(ActionOrigin {
            client_id: "client".into(),
            client_seq: 1,
        }),
    })?;
    assert!(matches!(
        change.envelopes[0].action,
        StateAction::ChatDraftChanged(ChatDraftChangedAction { draft: Some(_) })
    ));
    fixture.reopen()?;
    assert_eq!(snapshot_draft(&fixture, chat)?, Some(draft.clone()));
    let ActionReplay::Replay(replay) = fixture.store.replay_since(0)? else {
        return Err("expected replay".into());
    };
    assert!(replay.iter().any(|envelope| {
        matches!(envelope.action, StateAction::ChatDraftChanged(_))
            && envelope
                .origin
                .as_ref()
                .is_some_and(|origin| origin.client_seq == 1)
    }));
    assert!(matches!(
        fixture.store.change_draft(ChangeDraft {
            chat: chat.into(),
            draft: Some(message("invalid", "agent")?),
            turn_precondition: Default::default(),
            origin: None,
        }),
        Err(StoreError::Conflict(StoreConflict::InvalidDraft(_)))
    ));
    assert_eq!(snapshot_draft(&fixture, chat)?, Some(draft.clone()));

    let update = message("updated draft", "user")?;
    fixture.store.change_draft(ChangeDraft {
        chat: chat.into(),
        draft: Some(update),
        turn_precondition: Default::default(),
        origin: Some(ActionOrigin {
            client_id: "client".into(),
            client_seq: 2,
        }),
    })?;
    let started = fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action: start("sent")?,
        origin: None,
        legacy_default_selection: catalog().default_selection,
    })?;
    assert!(matches!(
        started.envelopes.as_slice(),
        [
            ahp_types::actions::ActionEnvelope {
                action: StateAction::ChatTurnStarted(_),
                ..
            },
            ahp_types::actions::ActionEnvelope {
                action: StateAction::ChatDraftChanged(ChatDraftChangedAction { draft: None }),
                ..
            },
            ..
        ]
    ));
    assert_eq!(snapshot_draft(&fixture, chat)?, None);
    Ok(())
}

fn message(text: &str, kind: &str) -> Result<Message, serde_json::Error> {
    serde_json::from_value(json!({ "text": text, "origin": { "kind": kind } }))
}

fn snapshot_draft(
    fixture: &Fixture,
    chat: &str,
) -> Result<Option<Message>, Box<dyn std::error::Error>> {
    let snapshot = fixture.store.snapshot(SnapshotRequest {
        channel: chat.into(),
        model_catalog: catalog(),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Chat(chat) = snapshot.state else {
        return Err("expected chat snapshot".into());
    };
    Ok(chat.draft)
}
