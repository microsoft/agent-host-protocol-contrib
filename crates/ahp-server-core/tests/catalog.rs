use ahp_server_core::{
    ActionReplay, ChangeChat, ChatMutation, CreateChat, CreateSession, FinishTurn, HostStore,
    MarkSessionReady, MemoryHostStore, ModelCatalog, SnapshotRequest, ToolCallTransition,
    TurnStart,
};
use ahp_types::actions::StateAction;
use ahp_types::state::{ChatState, SessionState, SessionStatus, SnapshotState};
use serde_json::{json, Value};

const SESSION: &str = "ahp-session:/catalog";
const CHAT: &str = "ahp-chat:/catalog";

fn store() -> MemoryHostStore {
    let store = MemoryHostStore::new();
    store
        .create_session(CreateSession {
            session: SESSION.into(),
            default_chat: CHAT.into(),
            provider: "openai".into(),
            workspace: "file:///workspace".into(),
            model_selection: ModelCatalog::single("test").default_selection,
        })
        .expect("create");
    store
        .mark_session_ready(MarkSessionReady {
            session: SESSION.into(),
        })
        .expect("ready");
    store
}

fn state(store: &dyn HostStore, uri: &str) -> SnapshotState {
    store
        .snapshot(SnapshotRequest {
            channel: uri.into(),
            model_catalog: ModelCatalog::single("test"),
            server_tools: Vec::new(),
        })
        .expect("snapshot")
        .state
}

fn session(store: &dyn HostStore) -> SessionState {
    match state(store, SESSION) {
        SnapshotState::Session(state) => Some(*state),
        _ => None,
    }
    .expect("session")
}

fn chat(store: &dyn HostStore, uri: &str) -> ChatState {
    match state(store, uri) {
        SnapshotState::Chat(state) => Some(*state),
        _ => None,
    }
    .expect("chat")
}

fn start(store: &dyn HostStore) {
    store
        .start_turn(TurnStart {
            chat: CHAT.into(),
            action: serde_json::from_value(json!({
                "turnId":"turn", "startedAt":"2026-10-24T00:00:00.000Z",
                "message":{"text":"hello", "origin":{"kind":"user"}},
            }))
            .expect("start action"),
            origin: None,
            legacy_default_selection: None,
        })
        .expect("start");
}

fn assert_projection(store: &dyn HostStore, uri: &str) {
    let owner = session(store);
    let summary = owner
        .chats
        .iter()
        .find(|summary| summary.resource == uri)
        .expect("catalog entry");
    let expected = serde_json::to_value(summary).expect("summary JSON");
    let actual = serde_json::to_value(chat(store, uri)).expect("chat JSON");
    for (key, value) in expected.as_object().expect("summary object") {
        assert_eq!(actual.get(key), Some(value), "summary field {key}");
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
            expected.get(key),
            actual.get(key),
            "optional summary field {key}"
        );
    }
    let summary = store.session_summary(SESSION).expect("root summary");
    assert_eq!(summary.default_chat.as_deref(), Some(CHAT));
    let compact = summary.chats.expect("compact catalog");
    assert_eq!(compact.len(), owner.chats.len());
    for (compact, full) in compact.iter().zip(&owner.chats) {
        assert_eq!(compact.resource, full.resource);
        assert_eq!(compact.status, Some(full.status));
    }
}

#[test]
fn read_flags_are_chat_local_and_project_into_both_catalogs() {
    let store = store();
    store
        .create_chat(CreateChat {
            session: SESSION.into(),
            chat: "ahp-chat:/sibling".into(),
        })
        .expect("sibling");
    let before = session(&store).status;
    for is_read in [true, false, true] {
        let changed = store
            .change_chat(ChangeChat {
                chat: CHAT.into(),
                mutation: ChatMutation::IsRead(is_read),
                origin: None,
            })
            .expect("read change");
        assert!(
            matches!(&changed.envelopes[0].action, StateAction::ChatIsReadChanged(action) if action.is_read == is_read)
        );
        assert_eq!(
            chat(&store, CHAT).status & SessionStatus::IsRead.bits() != 0,
            is_read
        );
        assert_eq!(session(&store).status, before);
        assert_eq!(
            chat(&store, "ahp-chat:/sibling").status & SessionStatus::IsRead.bits(),
            0
        );
        assert_projection(&store, CHAT);
    }
    start(&store);
    assert_eq!(chat(&store, CHAT).status & SessionStatus::IsRead.bits(), 0);
    assert_projection(&store, CHAT);
    store
        .finish_turn(FinishTurn::Complete {
            chat: CHAT.into(),
            turn_id: "turn".into(),
            model_cursor: None,
        })
        .expect("complete");
    assert_projection(&store, CHAT);
}

#[test]
fn confirmations_are_self_sufficient_and_removed_atomically_with_cancellation() {
    let store = store();
    start(&store);
    store.start_tool(CHAT, "turn", serde_json::from_value(json!({
        "kind":"toolCall", "toolCall":{"status":"streaming", "toolCallId":"tool", "toolName":"run_shell", "displayName":"Shell"},
    })).expect("streaming"), serde_json::from_value(json!({
        "type":"chat/toolCallStart", "turnId":"turn", "toolCallId":"tool", "toolName":"run_shell", "displayName":"Shell",
    })).expect("tool start")).expect("start tool");
    let pending = json!({
        "status":"pending-confirmation", "toolCallId":"tool", "toolName":"run_shell", "displayName":"Shell",
        "invocationMessage":"Run shell", "toolInput":"echo hello", "options":[{"id":"deny", "label":"Deny", "kind":"deny"}],
    });
    let transition = store.transition_tool(ToolCallTransition {
        chat: CHAT.into(), turn_id: "turn".into(), tool_call_id: "tool".into(),
        next_state: serde_json::from_value(pending.clone()).expect("pending"), actions: vec![serde_json::from_value(json!({
            "type":"chat/toolCallReady", "turnId":"turn", "toolCallId":"tool", "invocationMessage":"Run shell", "toolInput":"echo hello",
            "options":[{"id":"deny", "label":"Deny", "kind":"deny"}],
        })).expect("ready")], origin: None, allow_command: None,
    }).expect("pending confirmation");
    let aggregate = session(&store).input_needed.expect("aggregate");
    let request = serde_json::to_value(&aggregate[0]).expect("request");
    assert_eq!(request["toolCall"], pending);
    assert_eq!(request["chat"], CHAT);
    assert_eq!(request["turnId"], "turn");
    assert!(transition.envelopes.iter().any(|event| matches!(&event.action, StateAction::SessionInputNeededSet(action) if action.request == aggregate[0])));
    assert_projection(&store, CHAT);
    let since = store.server_sequence().expect("sequence");
    let cancelled = store
        .finish_turn(FinishTurn::Cancelled {
            chat: CHAT.into(),
            turn_id: "turn".into(),
            origin: None,
        })
        .expect("cancel");
    assert!(session(&store).input_needed.is_none());
    assert!(cancelled.envelopes.iter().any(|event| matches!(&event.action, StateAction::SessionInputNeededRemoved(action) if Value::String(action.id.clone()) == request["id"])));
    assert_eq!(
        store.replay_since(since).expect("replay"),
        ActionReplay::Replay(cancelled.envelopes)
    );
}

#[test]
fn disposal_removes_every_catalog_chat_before_the_root_count_update() {
    let store = store();
    store
        .create_chat(CreateChat {
            session: SESSION.into(),
            chat: "ahp-chat:/sibling".into(),
        })
        .expect("sibling");
    let before = session(&store).chats;
    let disposed = store.dispose_session(SESSION).expect("dispose");
    for (index, summary) in before.iter().enumerate() {
        assert_eq!(disposed.envelopes[index].channel, SESSION);
        assert!(
            matches!(&disposed.envelopes[index].action, StateAction::SessionChatRemoved(action) if action.chat == summary.resource)
        );
    }
    assert!(matches!(
        disposed.envelopes.last().expect("root action").action,
        StateAction::RootActiveSessionsChanged(_)
    ));
}

#[test]
fn result_confirmations_and_client_execution_preserve_self_sufficient_requests_and_queue_order() {
    use ahp_server_core::catalog::{
        chat_input_requests, input_request_actions, updated_input_requests,
    };
    use ahp_types::state::{ResponsePart, SessionInputRequest};

    let running: ResponsePart = serde_json::from_value(json!({
        "kind":"toolCall", "toolCall":{
            "status":"running", "toolCallId":"tool", "toolName":"client_tool", "displayName":"Client tool",
            "invocationMessage":"Execute", "confirmed":"not-needed", "toolInput":"{}",
            "contributor":{"kind":"client", "clientId":"executor"},
        },
    })).expect("client execution");
    let result: ResponsePart = serde_json::from_value(json!({
        "kind":"toolCall", "toolCall":{
            "status":"pending-result-confirmation", "toolCallId":"tool", "toolName":"server_tool", "displayName":"Server tool",
            "invocationMessage":"Execute", "confirmed":"not-needed", "success":true, "pastTenseMessage":"Executed",
        },
    })).expect("result confirmation");
    let first = chat_input_requests("ahp-chat:/second", "turn", &[running]);
    let second = chat_input_requests("ahp-chat:/first", "turn", &[result]);
    let request = match &first[0] {
        SessionInputRequest::ToolClientExecution(request) => Some(request),
        _ => None,
    }
    .expect("client execution");
    assert_eq!(request.client_id, "executor");
    assert_eq!(request.tool_call.tool_call_id, "tool");
    assert_eq!(
        serde_json::to_value(&first[0]).expect("request")["toolCall"]["status"],
        "running"
    );
    assert_eq!(
        serde_json::to_value(&second[0]).expect("request")["toolCall"]["status"],
        "pending-result-confirmation"
    );
    let before = [first.clone(), second.clone()].concat();
    let derived = [second, first].concat();
    assert_eq!(updated_input_requests(&before, &derived), before);
    assert!(input_request_actions(&before, &derived).is_empty());
    let removed = input_request_actions(&before, &[]);
    assert_eq!(removed.len(), 2);
    assert!(removed
        .iter()
        .all(|action| matches!(action, StateAction::SessionInputNeededRemoved(_))));
}
