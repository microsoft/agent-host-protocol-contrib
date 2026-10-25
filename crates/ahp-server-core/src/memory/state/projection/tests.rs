use ahp_types::state::{
    Message, ResponsePart, SessionStatus, SnapshotState, ToolCallPendingResultConfirmationState,
    ToolCallResponsePart, ToolCallState, TurnState,
};
use serde_json::json;

use super::super::super::super::presentation::ToolPresentation;
use super::super::{Active, Chat, Data, Session, StoredTurn};
use crate::store::SnapshotRequest;

fn message() -> Result<Message, String> {
    serde_json::from_value(json!({
        "text": "test", "origin": { "kind": "user" }
    }))
    .map_err(|error| error.to_string())
}

fn pending_result() -> ToolCallState {
    ToolCallState::PendingResultConfirmation(ToolCallPendingResultConfirmationState {
        tool_call_id: "result-confirmation".into(),
        tool_name: "tool".into(),
        display_name: "Tool".into(),
        intention: None,
        contributor: None,
        meta: None,
        invocation_message: "Review result".into(),
        tool_input: None,
        success: true,
        past_tense_message: "Ran tool".into(),
        content: None,
        structured_content: None,
        error: None,
        confirmed: ahp_types::state::ToolCallConfirmationReason::NotNeeded,
        selected_option: None,
    })
}

fn pending_chat(uri: &str, session: &str) -> Result<Chat, String> {
    let parameter_confirmation =
        ToolPresentation::new("parameter-confirmation".into(), "tool".into(), "{}".into())
            .map_err(|error| error.to_string())?
            .pending_state(Vec::new());
    let active = Active {
        id: "active".into(),
        started_at: "2026-01-01T00:00:00.000Z".into(),
        started_millis: 0,
        message: message()?,
        response_parts: vec![
            ResponsePart::ToolCall(Box::new(ToolCallResponsePart {
                tool_call: parameter_confirmation,
            })),
            ResponsePart::ToolCall(Box::new(ToolCallResponsePart {
                tool_call: pending_result(),
            })),
        ],
    };
    let mut chat = Chat::new(
        uri.into(),
        session.into(),
        "2026-01-01T00:00:01.000Z".into(),
    );
    chat.active = Some(active);
    Ok(chat)
}

fn errored_chat(uri: &str, session: &str) -> Result<Chat, String> {
    let mut chat = Chat::new(
        uri.into(),
        session.into(),
        "2026-01-01T00:00:02.000Z".into(),
    );
    chat.turns.push(StoredTurn {
        id: "failed".into(),
        started_at: "2026-01-01T00:00:00.000Z".into(),
        duration: 1,
        message: message()?,
        response_parts: Vec::new(),
        state: TurnState::Error,
        error: None,
    });
    Ok(chat)
}

fn data_with_multiple_chats() -> Result<Data, String> {
    let session_uri = "ahp-session:/session";
    let idle = Chat::new(
        "ahp-chat:/default".into(),
        session_uri.into(),
        "2026-01-01T00:00:00.000Z".into(),
    );
    let pending = pending_chat("ahp-chat:/pending", session_uri)?;
    let error = errored_chat("ahp-chat:/error", session_uri)?;
    let mut session = Session::new(
        session_uri.into(),
        "openai".into(),
        "file:///workspace".into(),
        "2026-01-01T00:00:00.000Z".into(),
        crate::ModelCatalog::single("model").default_selection,
    );
    session.ready = true;
    session.chats = vec![idle.uri.clone(), pending.uri.clone(), error.uri.clone()];
    let mut data = Data::default();
    data.sessions.insert(session.uri.clone(), session);
    data.chats.insert(idle.uri.clone(), idle);
    data.chats.insert(pending.uri.clone(), pending);
    data.chats.insert(error.uri.clone(), error);
    Ok(data)
}

#[test]
fn active_confirmation_states_project_input_needed_chat_snapshots() -> Result<(), String> {
    let data = data_with_multiple_chats()?;
    let snapshot = data
        .snapshot(SnapshotRequest {
            channel: "ahp-chat:/pending".into(),
            model_catalog: crate::ModelCatalog::single("model"),
            server_tools: Vec::new(),
        })
        .map_err(|error| error.to_string())?;
    let SnapshotState::Chat(chat) = snapshot.state else {
        return Err("expected chat snapshot".into());
    };
    assert!(SessionStatus::from_bits(chat.status).contains(SessionStatus::InputNeeded));
    assert_eq!(chat.activity, None);
    Ok(())
}

#[test]
fn session_snapshot_and_summary_promote_input_and_error_from_any_chat() -> Result<(), String> {
    let data = data_with_multiple_chats()?;
    let summary = data.session_summary(
        data.session("ahp-session:/session")
            .map_err(|error| error.to_string())?,
    );
    let summary_status = SessionStatus::from_bits(summary.status);
    assert!(summary_status.contains(SessionStatus::InputNeeded));
    assert!(summary_status.contains(SessionStatus::Error));
    let snapshot = data
        .snapshot(SnapshotRequest {
            channel: "ahp-session:/session".into(),
            model_catalog: crate::ModelCatalog::single("model"),
            server_tools: Vec::new(),
        })
        .map_err(|error| error.to_string())?;
    let SnapshotState::Session(session) = snapshot.state else {
        return Err("expected session snapshot".into());
    };
    let state_status = SessionStatus::from_bits(session.status);
    assert!(state_status.contains(SessionStatus::InputNeeded));
    assert!(state_status.contains(SessionStatus::Error));
    let pending = session
        .chats
        .iter()
        .find(|chat| chat.resource == "ahp-chat:/pending")
        .ok_or_else(|| "missing pending chat summary".to_owned())?;
    assert!(SessionStatus::from_bits(pending.status).contains(SessionStatus::InputNeeded));
    Ok(())
}
