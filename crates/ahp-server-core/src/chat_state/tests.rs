use ahp_types::actions::{
    ChatPendingMessageRemovedAction, ChatPendingMessageSetAction, ChatQueuedMessagesReorderedAction,
};
use ahp_types::state::PendingMessageKind::{Queued, Steering};
use serde_json::json;

use super::{ChatMutation, PendingMessages};

fn set(id: &str, kind: ahp_types::state::PendingMessageKind, text: &str) -> ChatMutation {
    ChatMutation::PendingSet(ChatPendingMessageSetAction {
        id: id.into(),
        kind,
        message: serde_json::from_value(json!({ "text": text, "origin": { "kind": "user" } }))
            .expect("test message"),
    })
}

#[test]
fn queued_entries_upsert_in_place_and_reorder_without_changing_steering() {
    let mut state = PendingMessages::default();
    for id in ["q1", "q2", "q3"] {
        state.apply(&set(id, Queued, id)).expect("queue input");
    }
    state
        .apply(&set("steer", Steering, "steer"))
        .expect("set steering");
    state.apply(&set("q1", Queued, "updated")).expect("upsert");
    assert_eq!(state.entries[0].message.text, "updated");
    state
        .apply(&ChatMutation::Reordered(
            ChatQueuedMessagesReorderedAction {
                order: vec!["q3".into(), "missing".into(), "q3".into()],
            },
        ))
        .expect("partial reorder");
    assert_eq!(
        state
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["q3", "q1", "q2"]
    );
    assert_eq!(
        state.steering.as_ref().map(|entry| entry.id.as_str()),
        Some("steer")
    );
    let entries = serde_json::to_value(&state.entries).expect("queued JSON");
    assert!(entries[0].get("kind").is_none());
}

#[test]
fn steering_replaces_its_slot_and_removals_address_independent_kinds() {
    let mut state = PendingMessages::default();
    state.apply(&set("first", Steering, "first")).expect("set");
    state
        .apply(&set("second", Steering, "second"))
        .expect("replace steering");
    assert!(state.entries.is_empty());
    assert_eq!(
        state.steering.as_ref().map(|entry| entry.id.as_str()),
        Some("second")
    );
    let before = state.clone();
    assert!(state
        .apply(&ChatMutation::PendingRemoved(
            ChatPendingMessageRemovedAction {
                id: "second".into(),
                kind: Queued,
            }
        ))
        .is_err());
    assert_eq!(state, before);
    state
        .apply(&set("second", Queued, "independent queued input"))
        .expect("queue same id");
    state
        .apply(&ChatMutation::PendingRemoved(
            ChatPendingMessageRemovedAction {
                id: "second".into(),
                kind: Steering,
            },
        ))
        .expect("remove steering only");
    assert!(state.steering.is_none());
    assert_eq!(state.entries[0].id, "second");
    state
        .apply(&set("second", Steering, "steer again"))
        .expect("set steering");
    state
        .apply(&ChatMutation::PendingRemoved(
            ChatPendingMessageRemovedAction {
                id: "second".into(),
                kind: Queued,
            },
        ))
        .expect("remove queued input only");
    assert!(state.entries.is_empty());
    assert!(state.steering.is_some());
}

#[test]
fn turn_start_consumes_matching_ids_from_both_input_slots() {
    let mut state = PendingMessages::default();
    state.apply(&set("same", Queued, "queued")).expect("queue");
    state
        .apply(&set("same", Steering, "steering"))
        .expect("steer");
    assert_eq!(
        state
            .consume("same")
            .expect("consume queued input")
            .message
            .text,
        "queued"
    );
    assert!(state.entries.is_empty());
    assert!(state.steering.is_none());
    state
        .apply(&set("same", Queued, "new queued input"))
        .expect("reuse consumed id");
}
