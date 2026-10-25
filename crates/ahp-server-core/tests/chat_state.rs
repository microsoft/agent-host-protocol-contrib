use ahp_server_core::{
    ChangeChat, ChatMutation, CreateSession, FinishTurn, HostStore, MarkSessionReady,
    MemoryHostStore, ModelCatalog, ModelCursor, SnapshotRequest, TurnStart,
};
use ahp_types::actions::{
    ChatPendingMessageSetAction, ChatTruncatedAction, ChatTurnStartedAction, StateAction,
};
use ahp_types::state::{ChatState, PendingMessageKind, ResponsePart};
use serde_json::json;

const CHAT: &str = "ahp-chat:/chat-state";

fn store() -> MemoryHostStore {
    let store = MemoryHostStore::new();
    store
        .create_session(CreateSession {
            session: "ahp-session:/chat-state".into(),
            default_chat: CHAT.into(),
            provider: "openai".into(),
            workspace: "file:///workspace".into(),
            model_selection: ModelCatalog::single("test").default_selection,
        })
        .expect("create session");
    store
        .mark_session_ready(MarkSessionReady {
            session: "ahp-session:/chat-state".into(),
        })
        .expect("ready session");
    store
}

fn start(id: &str) -> TurnStart {
    TurnStart {
        chat: CHAT.into(),
        action: ChatTurnStartedAction {
            turn_id: id.into(),
            started_at: "2020-01-01T00:00:00.123Z".into(),
            message: serde_json::from_value(json!({ "text": id, "origin": { "kind": "user" } }))
                .expect("message"),
            queued_message_id: None,
            meta: None,
        },
        origin: None,
        legacy_default_selection: None,
    }
}

fn snapshot(store: &MemoryHostStore) -> ChatState {
    let snapshot = store
        .snapshot(SnapshotRequest {
            channel: CHAT.into(),
            model_catalog: ModelCatalog::single("test"),
            server_tools: Vec::new(),
        })
        .expect("snapshot");
    serde_json::from_value(serde_json::to_value(snapshot.state).expect("snapshot JSON"))
        .expect("chat snapshot")
}

#[test]
fn timestamps_and_truncated_model_history_follow_the_journal() {
    let store = store();
    store.start_turn(start("first")).expect("start first");
    assert_eq!(snapshot(&store).modified_at, "2020-01-01T00:00:00.123Z");
    store
        .append_text(CHAT, "first", "response-first", "answer")
        .expect("append text");
    assert_eq!(snapshot(&store).modified_at, "2020-01-01T00:00:00.123Z");
    let finished = store
        .finish_turn(FinishTurn::Complete {
            chat: CHAT.into(),
            turn_id: "first".into(),
            model_cursor: None,
        })
        .expect("finish first");
    let expected = chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00.123Z")
        .expect("timestamp")
        + chrono::TimeDelta::milliseconds(finished.value);
    assert_eq!(
        snapshot(&store).modified_at,
        expected.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    );
    store.start_turn(start("second")).expect("start second");
    store
        .finish_turn(FinishTurn::Complete {
            chat: CHAT.into(),
            turn_id: "second".into(),
            model_cursor: Some(ModelCursor {
                scope: "scope".into(),
                response_id: "response".into(),
                turn_id: "second".into(),
            }),
        })
        .expect("finish second");
    store
        .change_chat(ChangeChat {
            chat: CHAT.into(),
            mutation: ChatMutation::Truncated(ChatTruncatedAction {
                turn_id: Some("first".into()),
            }),
            origin: None,
        })
        .expect("truncate");
    let next = store
        .start_turn(start("third"))
        .expect("start after truncation");
    assert_eq!(
        next.value
            .history
            .iter()
            .map(|turn| turn.id.as_str())
            .collect::<Vec<_>>(),
        ["first"]
    );
    assert!(next.value.model_cursor.is_none());
}

#[test]
fn queued_turn_consumption_commits_removal_before_start() {
    let store = store();
    let request = start("queued-turn");
    store
        .change_chat(ChangeChat {
            chat: CHAT.into(),
            origin: None,
            mutation: ChatMutation::PendingSet(ChatPendingMessageSetAction {
                id: "queued".into(),
                kind: PendingMessageKind::Queued,
                message: request.action.message.clone(),
            }),
        })
        .expect("enqueue");
    let mut request = request;
    request.action.queued_message_id = Some("queued".into());
    let started = store.start_turn(request).expect("consume queued turn");
    assert!(
        matches!(&started.envelopes[0].action, StateAction::ChatPendingMessageRemoved(action) if action.id == "queued")
    );
    assert!(
        matches!(&started.envelopes[1].action, StateAction::ChatTurnStarted(action) if action.queued_message_id.as_deref() == Some("queued"))
    );
    assert!(store
        .pending_messages(CHAT)
        .expect("pending input")
        .entries
        .is_empty());
    assert!(snapshot(&store).queued_messages.is_none());
}

#[test]
fn implicit_tool_cancellation_preserves_input_without_inventing_a_reason_message() {
    let store = store();
    store.start_turn(start("cancelled")).expect("start");
    let part: ResponsePart = serde_json::from_value(json!({
        "kind": "toolCall", "toolCall": {
            "status": "running", "toolCallId": "tool", "toolName": "example", "displayName": "Example",
            "invocationMessage": "Running example", "toolInput": "input", "confirmed": "not-needed"
        }
    })).expect("tool part");
    store
        .start_tool(
            CHAT,
            "cancelled",
            part,
            StateAction::Unknown(json!({ "type": "chat/toolCallStart" })),
        )
        .expect("persist tool");
    store
        .finish_turn(FinishTurn::Cancelled {
            chat: CHAT.into(),
            turn_id: "cancelled".into(),
            origin: None,
        })
        .expect("cancel turn");
    let value = serde_json::to_value(snapshot(&store)).expect("JSON snapshot");
    assert_eq!(
        value["turns"][0]["responseParts"][1]["toolCall"],
        json!({
            "status": "cancelled", "toolCallId": "tool", "toolName": "example", "displayName": "Example",
            "invocationMessage": "Running example", "toolInput": "input", "reason": "skipped"
        })
    );
}
