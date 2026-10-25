use std::path::PathBuf;

use ahp_server_core::{
    ActionReplay, ChangeChat, ChangeClientTool, ChatMutation, ClientToolMutation, CreateSession,
    FinishTurn, HostStore, MarkSessionReady, ModelCatalog, SnapshotRequest, ToolCallTransition,
    TurnStart,
};
use ahp_types::actions::{ActionOrigin, StateAction};
use ahp_types::state::{ResponsePart, SnapshotState, ToolCallState};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::RedbHostStore;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Fixture {
    path: PathBuf,
    store: RedbHostStore,
}

impl Fixture {
    fn new() -> Result<Self> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("ahp-context-{}.redb", Uuid::new_v4()));
        let store = RedbHostStore::open(&path)?;
        Ok(Self { path, store })
    }

    fn ready(&self, id: &str) -> Result {
        self.store.create_session(CreateSession {
            session: format!("ahp-session:/{id}"),
            default_chat: format!("ahp-chat:/{id}"),
            provider: "test".into(),
            workspace: "file:///workspace".into(),
            model_selection: ModelCatalog::single("test").default_selection,
        })?;
        self.store.mark_session_ready(MarkSessionReady {
            session: format!("ahp-session:/{id}"),
        })?;
        Ok(())
    }

    fn start(
        &self,
        chat: &str,
        id: &str,
        attachments: Value,
    ) -> Result<ahp_server_core::StartedTurn> {
        Ok(self.store.start_turn(TurnStart {chat:format!("ahp-chat:/{chat}"),action:serde_json::from_value(json!({
            "turnId":id,"startedAt":"2026-10-24T00:00:00.000Z","message":{"text":format!("Remember this phrase: \"{id}\""),"origin":{"kind":"user"},"attachments":attachments}
        }))?,origin:None,legacy_default_selection:None})?.value)
    }

    fn finish(&self, chat: &str, id: &str) -> Result {
        self.store.finish_turn(FinishTurn::Complete {
            chat: format!("ahp-chat:/{chat}"),
            turn_id: id.into(),
            model_cursor: None,
        })?;
        Ok(())
    }

    fn reopen(&mut self) -> Result {
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

#[test]
fn attachment_materialization_survives_reopen_and_source_pruning() -> Result {
    let mut fixture = Fixture::new()?;
    fixture.ready("source")?;
    fixture.ready("target")?;
    let sequence = fixture.store.server_sequence()?;
    assert!(fixture
        .start(
            "target",
            "invalid",
            json!([{"type":"chat","resource":"ahp-chat:/missing","label":"Missing"}])
        )
        .is_err());
    assert_eq!(fixture.store.server_sequence()?, sequence);
    fixture.start("source", "first", json!([]))?;
    fixture.finish("source", "first")?;
    let accepted = fixture.start(
        "target",
        "attached",
        json!([{"type":"chat","resource":"ahp-chat:/source","label":"Pinned"}]),
    )?;
    fixture.finish("target", "attached")?;
    fixture.store.change_chat(ChangeChat {
        chat: "ahp-chat:/source".into(),
        mutation: ChatMutation::Truncated(serde_json::from_value(json!({}))?),
        origin: None,
    })?;
    let sequence = fixture.store.server_sequence()?;
    fixture.reopen()?;
    assert_eq!(fixture.store.server_sequence()?, sequence);
    let next = fixture.start("target", "next", json!([]))?;
    assert_eq!(
        next.materialized_attachments["attached"],
        accepted.materialized_attachments["attached"]
    );
    assert_eq!(
        next.materialized_attachments["attached"][0].message.text,
        "Remember this phrase: \"first\""
    );
    Ok(())
}

#[test]
fn client_tool_updates_are_owned_atomic_replayable_and_durable() -> Result {
    let mut fixture = Fixture::new()?;
    fixture.ready("tools")?;
    fixture.start("tools", "turn", json!([]))?;
    let contributor = json!({"kind":"client","clientId":"owner"});
    fixture.store.start_tool("ahp-chat:/tools","turn",serde_json::from_value(json!({
        "kind":"toolCall","toolCall":{"status":"streaming","toolCallId":"call","toolName":"echo","displayName":"Echo","contributor":contributor}
    }))?,serde_json::from_value(json!({"type":"chat/toolCallStart","turnId":"turn","toolCallId":"call","toolName":"echo","displayName":"Echo","contributor":contributor}))?)?;
    fixture.store.transition_tool(ToolCallTransition {
        chat:"ahp-chat:/tools".into(),turn_id:"turn".into(),tool_call_id:"call".into(),
        next_state:serde_json::from_value(json!({"status":"running","toolCallId":"call","toolName":"echo","displayName":"Echo","contributor":contributor,"invocationMessage":"Echoing","confirmed":"not-needed"}))?,
        actions:vec![serde_json::from_value(json!({"type":"chat/toolCallReady","turnId":"turn","toolCallId":"call","invocationMessage":"Echoing","confirmed":"not-needed"}))?],origin:None,allow_command:None,
    })?;
    let before = fixture.store.server_sequence()?;
    let mut committed = Vec::new();
    for text in ["first", "replacement"] {
        let request = ChangeClientTool {
            chat: "ahp-chat:/tools".into(),
            turn_id: "turn".into(),
            tool_call_id: "call".into(),
            client_id: "owner".into(),
            mutation: ClientToolMutation::Content(serde_json::from_value(
                json!([{"type":"text","text":text}]),
            )?),
            origin: Some(ActionOrigin {
                client_id: "owner".into(),
                client_seq: 1,
            }),
        };
        let mut spoofed = request.clone();
        spoofed.origin.as_mut().expect("origin").client_id = "observer".into();
        let sequence = fixture.store.server_sequence()?;
        assert!(fixture.store.change_client_tool(spoofed).is_err());
        assert_eq!(fixture.store.server_sequence()?, sequence);
        committed.extend(fixture.store.change_client_tool(request)?.envelopes);
    }
    committed.extend(fixture.store.change_client_tool(ChangeClientTool {
        chat:"ahp-chat:/tools".into(),turn_id:"turn".into(),tool_call_id:"call".into(),client_id:"owner".into(),
        mutation:ClientToolMutation::Complete(serde_json::from_value(json!({"success":true,"pastTenseMessage":"Echoed","content":[{"type":"text","text":"final"}],"structuredContent":{"text":"final"}}))?),
        origin:Some(ActionOrigin {client_id:"owner".into(),client_seq:2}),
    })?.envelopes);
    committed.extend(
        fixture
            .store
            .finish_turn(FinishTurn::Complete {
                chat: "ahp-chat:/tools".into(),
                turn_id: "turn".into(),
                model_cursor: None,
            })?
            .envelopes,
    );
    fixture.reopen()?;
    let ActionReplay::Replay(actions) = fixture.store.replay_since(before)? else {
        return Err("replay required".into());
    };
    assert_eq!(actions, committed);
    assert!(actions
        .iter()
        .any(|action| matches!(&action.action, StateAction::ChatToolCallComplete(_))));
    let snapshot = fixture.store.snapshot(SnapshotRequest {
        channel: "ahp-chat:/tools".into(),
        model_catalog: ModelCatalog::single("test"),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Chat(chat) = snapshot.state else {
        return Err("chat required".into());
    };
    let completed = chat.turns[0]
        .response_parts
        .iter()
        .find_map(|part| match part {
            ResponsePart::ToolCall(tool) => match &tool.tool_call {
                ToolCallState::Completed(completed) => Some(completed),
                _ => None,
            },
            _ => None,
        })
        .ok_or("completed call required")?;
    assert!(completed.success);
    assert_eq!(
        serde_json::to_value(&completed.content)?,
        json!([{"type":"text","text":"final"}])
    );
    assert_eq!(
        completed
            .structured_content
            .as_ref()
            .ok_or("structured result")?["text"],
        "final"
    );
    Ok(())
}
