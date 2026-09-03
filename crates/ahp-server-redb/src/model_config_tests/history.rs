use ahp_server_core::{
    CreateSession, FinishTurn, HostStore, MarkSessionReady, ModelCursor, TurnStart,
};

use super::{catalog, selection, start, Fixture};

#[test]
fn started_turn_includes_persisted_history_after_restart() -> Result<(), Box<dyn std::error::Error>>
{
    let mut fixture = Fixture::new()?;
    let session = "ahp-session:/history-redb";
    let chat = "ahp-chat:/history-redb";
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
    fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action: start("first")?,
        origin: None,
        legacy_default_selection: catalog().default_selection,
    })?;
    fixture.store.finish_turn(FinishTurn::Complete {
        chat: chat.into(),
        turn_id: "first".into(),
        model_cursor: Some(ModelCursor {
            scope: "https://api.openai.com/v1/responses".into(),
            response_id: "resp_first".into(),
            turn_id: "first".into(),
        }),
    })?;
    fixture.reopen()?;

    let second = fixture.store.start_turn(TurnStart {
        chat: chat.into(),
        action: start("second")?,
        origin: None,
        legacy_default_selection: catalog().default_selection,
    })?;

    assert_eq!(second.value.history.len(), 1);
    assert_eq!(second.value.history[0].id, "first");
    assert_eq!(second.value.history[0].message.text, "test");
    assert_eq!(
        second.value.model_cursor,
        Some(ModelCursor {
            scope: "https://api.openai.com/v1/responses".into(),
            response_id: "resp_first".into(),
            turn_id: "first".into(),
        })
    );
    Ok(())
}
