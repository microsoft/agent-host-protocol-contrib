mod catalog;

use std::path::PathBuf;

use ahp_server_core::{
    ActionReplay, CreateChat, CreateSession, FinishTurn, HostStore, MarkSessionReady, ModelCatalog,
    SnapshotRequest, StoreConflict, StoreError, ToolCallTransition, TurnStart,
};
use ahp_types::actions::{
    ActionEnvelope, ActionOrigin, ChatToolCallDeltaAction, ChatToolCallReadyAction,
    ChatToolCallStartAction, ChatTurnStartedAction, StateAction,
};
use ahp_types::common::Uri;
use ahp_types::state::{
    ChatState, ErrorInfo, MarkdownResponsePart, ResponsePart, SessionState, SessionStatus,
    SnapshotState, ToolCallConfirmationReason, ToolCallResponsePart, ToolCallRunningState,
    ToolCallState, ToolCallStreamingState, ToolInput, TurnState,
};
use serde_json::json;
use uuid::Uuid;

use crate::tables::{CHATS, META, SESSIONS, SESSION_IDS};
use crate::{RedbHostStore, RedbStoreError};

struct TestStore {
    path: PathBuf,
    store: RedbHostStore,
}

impl TestStore {
    fn new() -> Result<Self, RedbStoreError> {
        let path = std::env::temp_dir().join(format!("ahp-server-redb-{}.redb", Uuid::new_v4()));
        Ok(Self {
            store: RedbHostStore::open(&path)?,
            path,
        })
    }

    fn store(&self) -> &RedbHostStore {
        &self.store
    }

    fn reopen(&mut self) -> Result<(), RedbStoreError> {
        self.store.close()?;
        self.store = RedbHostStore::open(&self.path)?;
        Ok(())
    }
}

impl Drop for TestStore {
    fn drop(&mut self) {
        let _ = self.store.close();
        for suffix in ["", ".lock"] {
            let candidate = format!("{}{suffix}", self.path.display());
            let _ = std::fs::remove_file(candidate);
        }
    }
}

fn create_ready(store: &RedbHostStore) -> Result<(Uri, Uri), StoreError> {
    let session: Uri = format!("ahp-session:/{}", Uuid::new_v4());
    let chat: Uri = format!("ahp-chat:/{}", Uuid::new_v4());
    store.create_session(CreateSession {
        session: session.clone(),
        default_chat: chat.clone(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: Some(default_selection()),
    })?;
    store.mark_session_ready(MarkSessionReady {
        session: session.clone(),
    })?;
    Ok((session, chat))
}

fn default_selection() -> ahp_types::state::ModelSelection {
    ModelCatalog::single("test-model")
        .default_selection
        .expect("single catalog has a default")
}

fn start(turn_id: &str) -> Result<ChatTurnStartedAction, serde_json::Error> {
    Ok(ChatTurnStartedAction {
        turn_id: turn_id.into(),
        started_at: "2026-08-30T12:00:00.000Z".into(),
        message: serde_json::from_value(json!({
            "text": "test",
            "origin": { "kind": "user" }
        }))?,
        meta: None,
        queued_message_id: None,
    })
}

fn streaming_part(id: &str) -> ResponsePart {
    ResponsePart::ToolCall(Box::new(ToolCallResponsePart {
        tool_call: ToolCallState::Streaming(ToolCallStreamingState {
            tool_call_id: id.into(),
            tool_name: "replace_string".into(),
            display_name: "Replace text".into(),
            intention: Some("Replace text".into()),
            contributor: None,
            meta: None,
            partial_input: None,
            invocation_message: Some("Replace text".into()),
        }),
    }))
}

fn tool_start_action(turn_id: &str, id: &str) -> StateAction {
    StateAction::ChatToolCallStart(ChatToolCallStartAction {
        turn_id: turn_id.into(),
        tool_call_id: id.into(),
        meta: None,
        tool_name: "replace_string".into(),
        display_name: "Replace text".into(),
        intention: Some("Replace text".into()),
        contributor: None,
    })
}

fn tool_delta_action(turn_id: &str, id: &str, delta: &str) -> StateAction {
    StateAction::ChatToolCallDelta(ChatToolCallDeltaAction {
        turn_id: turn_id.into(),
        tool_call_id: id.into(),
        meta: None,
        content: Some(delta.into()),
        invocation_message: None,
    })
}

fn running_state(id: &str) -> ToolCallState {
    ToolCallState::Running(ToolCallRunningState {
        tool_call_id: id.into(),
        tool_name: "replace_string".into(),
        display_name: "Replace text".into(),
        intention: Some("Replace text".into()),
        contributor: None,
        meta: None,
        invocation_message: "Replace text".into(),
        tool_input: Some(ToolInput::Inline("{}".into())),
        confirmed: ToolCallConfirmationReason::NotNeeded,
        selected_option: None,
        content: None,
    })
}

fn ready_action(turn_id: &str, id: &str) -> StateAction {
    StateAction::ChatToolCallReady(ChatToolCallReadyAction {
        turn_id: turn_id.into(),
        tool_call_id: id.into(),
        meta: None,
        contributor: None,
        intention: Some("Replace text".into()),
        invocation_message: "Replace text".into(),
        tool_input: Some(ToolInput::Inline("{}".into())),
        confirmation_title: None,
        risk_assessment: None,
        edits: None,
        editable: Some(false),
        confirmed: Some(ToolCallConfirmationReason::NotNeeded),
        options: None,
    })
}

fn chat_state(store: &RedbHostStore, chat: Uri) -> Result<ChatState, StoreError> {
    let snapshot = store.snapshot(SnapshotRequest {
        channel: chat,
        model_catalog: ModelCatalog::single("test-model"),
        server_tools: Vec::new(),
    })?;
    match snapshot.state {
        SnapshotState::Chat(chat) => Ok(*chat),
        _ => Err(StoreError::Backend("expected chat snapshot".into())),
    }
}

fn session_state(store: &RedbHostStore, session: Uri) -> Result<SessionState, StoreError> {
    let snapshot = store.snapshot(SnapshotRequest {
        channel: session,
        model_catalog: ModelCatalog::single("test-model"),
        server_tools: Vec::new(),
    })?;
    match snapshot.state {
        SnapshotState::Session(session) => Ok(*session),
        _ => Err(StoreError::Backend("expected session snapshot".into())),
    }
}

fn pending_confirmation_state(id: &str) -> Result<ToolCallState, serde_json::Error> {
    serde_json::from_value(json!({
        "status": "pending-confirmation",
        "toolCallId": id,
        "toolName": "replace_string",
        "displayName": "Replace text",
        "invocationMessage": "Replace text",
        "toolInput": "{}"
    }))
}

fn pending_result_confirmation_state(id: &str) -> Result<ToolCallState, serde_json::Error> {
    serde_json::from_value(json!({
        "status": "pending-result-confirmation",
        "toolCallId": id,
        "toolName": "replace_string",
        "displayName": "Replace text",
        "invocationMessage": "Replace text",
        "toolInput": "{}",
        "success": true,
        "pastTenseMessage": "Replaced text",
        "confirmed": "not-needed"
    }))
}

fn set_chat_modified_at(
    store: &RedbHostStore,
    chat_uri: &str,
    modified_at: &str,
) -> Result<(), StoreError> {
    store.test_write(|write| {
        let mut chat = crate::sessions::load_chat(write, chat_uri)?
            .ok_or_else(|| StoreError::Backend("test chat is missing".into()))?;
        chat.modified_at = modified_at.into();
        crate::sessions::write_chat(write, &chat)
    })
}

fn replace_active_tool_state(
    store: &RedbHostStore,
    chat_uri: &str,
    tool_call_id: &str,
    next_state: ToolCallState,
) -> Result<(), StoreError> {
    store.test_write(|write| {
        let mut chat = crate::sessions::load_chat(write, chat_uri)?
            .ok_or_else(|| StoreError::Backend("test chat is missing".into()))?;
        let active = chat
            .active
            .as_mut()
            .ok_or_else(|| StoreError::Backend("test active turn is missing".into()))?;
        let tool = active
            .response_parts
            .iter_mut()
            .find_map(|part| match part {
                ResponsePart::ToolCall(tool)
                    if crate::types::tool_call_id(&tool.tool_call) == tool_call_id =>
                {
                    Some(tool)
                }
                _ => None,
            })
            .ok_or_else(|| StoreError::Backend("test tool is missing".into()))?;
        tool.tool_call = next_state;
        crate::sessions::write_chat(write, &chat)
    })
}

fn journal(store: &RedbHostStore) -> Result<Vec<ActionEnvelope>, StoreError> {
    match store.replay_since(0)? {
        ActionReplay::Replay(actions) => Ok(actions),
        ActionReplay::SnapshotRequired => Err(StoreError::Backend("unexpected replay gap".into())),
    }
}

fn assert_sequences(envelopes: &[ActionEnvelope]) {
    for pair in envelopes.windows(2) {
        assert_eq!(pair[1].server_seq, pair[0].server_seq + 1);
    }
}

fn reduce_completed_markdown(actions: &[ActionEnvelope]) -> Vec<(String, String)> {
    let mut active = None;
    let mut completed = Vec::new();
    for envelope in actions {
        match &envelope.action {
            StateAction::ChatTurnStarted(action) => {
                active = Some((action.turn_id.clone(), String::new()));
            }
            StateAction::ChatDelta(action) => {
                if let Some((turn_id, markdown)) = &mut active {
                    if *turn_id == action.turn_id {
                        markdown.push_str(&action.content);
                    }
                }
            }
            StateAction::ChatTurnComplete(_)
            | StateAction::ChatTurnCancelled(_)
            | StateAction::ChatError(_) => {
                if let Some(turn) = active.take() {
                    completed.push(turn);
                }
            }
            _ => {}
        }
    }
    completed
}

#[test]
fn catalogue_transitions_are_journaled_and_snapshots_are_typed(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = TestStore::new()?;
    let store = fixture.store();
    let session: Uri = "ahp-session:/catalogue".into();
    let chat: Uri = "ahp-chat:/catalogue-default".into();
    let created = store.create_session(CreateSession {
        session: session.clone(),
        default_chat: chat.clone(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: Some(default_selection()),
    })?;
    assert_eq!(created.envelopes.len(), 2);
    assert!(matches!(
        store
            .snapshot(SnapshotRequest {
                channel: "ahp-root://".into(),
                model_catalog: ModelCatalog::single("model"),
                server_tools: Vec::new(),
            })?
            .state,
        SnapshotState::Root(_)
    ));
    assert!(matches!(
        store
            .snapshot(SnapshotRequest {
                channel: session.clone(),
                model_catalog: ModelCatalog::single("model"),
                server_tools: Vec::new(),
            })?
            .state,
        SnapshotState::Session(_)
    ));
    let before = store.server_sequence()?;
    assert!(matches!(
        store.create_chat(CreateChat {
            session: session.clone(),
            chat: "ahp-chat:/not-ready".into(),
        }),
        Err(StoreError::Conflict(StoreConflict::SessionNotReady(_)))
    ));
    assert_eq!(store.server_sequence()?, before);
    let ready = store.mark_session_ready(MarkSessionReady {
        session: session.clone(),
    })?;
    assert_eq!(ready.envelopes.len(), 1);
    let extra = store.create_chat(CreateChat {
        session: session.clone(),
        chat: "ahp-chat:/catalogue-extra".into(),
    })?;
    assert_eq!(extra.envelopes.len(), 1);
    assert_eq!(store.list_sessions()?.len(), 1);
    let disposed = store.dispose_session(&session)?;
    assert_eq!(
        disposed.value.chats,
        vec![chat, "ahp-chat:/catalogue-extra".into()]
    );
    assert_eq!(disposed.envelopes.len(), 3);
    for (event, chat) in disposed.envelopes.iter().zip(&disposed.value.chats) {
        assert_eq!(event.channel, session);
        assert!(
            matches!(&event.action, StateAction::SessionChatRemoved(action) if action.chat == *chat)
        );
    }
    assert!(matches!(
        store.snapshot(SnapshotRequest {
            channel: session,
            model_catalog: ModelCatalog::single("model"),
            server_tools: Vec::new(),
        }),
        Err(StoreError::Conflict(StoreConflict::UnknownChannel(_)))
    ));
    assert_sequences(&journal(store)?);
    Ok(())
}

#[test]
fn turn_transitions_commit_state_journal_origin_and_rejection(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = TestStore::new()?;
    let store = fixture.store();
    let (session, chat) = create_ready(store)?;
    let origin = Some(ActionOrigin {
        client_id: "client".into(),
        client_seq: 4,
    });
    let started = store.start_turn(TurnStart {
        chat: chat.clone(),
        action: start("turn-1")?,
        origin: origin.clone(),
        legacy_default_selection: Some(default_selection()),
    })?;
    assert_eq!(started.envelopes.len(), 4);
    assert_eq!(started.envelopes[0].origin, origin);
    store.append_text(&chat, "turn-1", "response-turn-1", "Hello ")?;
    store.start_tool(
        &chat,
        "turn-1",
        streaming_part("tool-1"),
        tool_start_action("turn-1", "tool-1"),
    )?;
    store.append_tool_delta(
        &chat,
        "turn-1",
        "tool-1",
        r#"{"path":"note.txt"}"#,
        tool_delta_action("turn-1", "tool-1", r#"{"path":"note.txt"}"#),
    )?;
    store.transition_tool(ToolCallTransition {
        chat: chat.clone(),
        turn_id: "turn-1".into(),
        tool_call_id: "tool-1".into(),
        next_state: running_state("tool-1"),
        actions: vec![ready_action("turn-1", "tool-1")],
        origin: None,
        allow_command: None,
    })?;
    let finished = store.finish_turn(FinishTurn::Complete {
        chat: chat.clone(),
        turn_id: "turn-1".into(),
        model_cursor: None,
    })?;
    assert_eq!(finished.envelopes.len(), 2);
    let state = chat_state(store, chat.clone())?;
    assert!(state.active_turn.is_none());
    assert_eq!(state.turns.len(), 1);
    let ResponsePart::Markdown(MarkdownResponsePart { content, .. }) =
        &state.turns[0].response_parts[0]
    else {
        return Err(std::io::Error::other("missing markdown response part").into());
    };
    assert_eq!(content, "Hello ");
    assert_eq!(
        reduce_completed_markdown(&journal(store)?),
        vec![("turn-1".into(), "Hello ".into())]
    );
    store.reject_action(
        session,
        StateAction::ChatTurnStarted(start("ignored")?),
        None,
        "not allowed".into(),
    )?;
    let replay = journal(store)?;
    assert_sequences(&replay);
    let rejection = replay
        .last()
        .ok_or_else(|| std::io::Error::other("missing rejection"))?;
    assert_eq!(rejection.rejection_reason.as_deref(), Some("not allowed"));
    Ok(())
}

#[test]
fn conflicts_and_invalid_transitions_leave_state_and_journal_unchanged(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = TestStore::new()?;
    let store = fixture.store();
    let (_, chat) = create_ready(store)?;
    store.start_turn(TurnStart {
        chat: chat.clone(),
        action: start("turn-1")?,
        origin: None,
        legacy_default_selection: Some(default_selection()),
    })?;
    let before_sequence = store.server_sequence()?;
    let before_state = serde_json::to_value(chat_state(store, chat.clone())?)?;
    assert!(matches!(
        store.start_turn(TurnStart {
            chat: chat.clone(),
            action: start("turn-2")?,
            origin: None,
            legacy_default_selection: Some(default_selection()),
        }),
        Err(StoreError::Conflict(StoreConflict::ActiveTurn(_)))
    ));
    assert!(matches!(
        store.append_text(&chat, "wrong-turn", "response-turn-1", "ignored"),
        Err(StoreError::Conflict(StoreConflict::TurnMismatch { .. }))
    ));
    assert!(matches!(
        store.append_tool_delta(
            &chat,
            "turn-1",
            "missing",
            "ignored",
            tool_delta_action("turn-1", "missing", "ignored"),
        ),
        Err(StoreError::Conflict(StoreConflict::ToolNotFound { .. }))
    ));
    store.start_tool(
        &chat,
        "turn-1",
        streaming_part("tool-1"),
        tool_start_action("turn-1", "tool-1"),
    )?;
    let after_tool_sequence = store.server_sequence()?;
    let after_tool_state = serde_json::to_value(chat_state(store, chat.clone())?)?;
    assert!(matches!(
        store.start_tool(
            &chat,
            "turn-1",
            streaming_part("tool-1"),
            tool_start_action("turn-1", "tool-1"),
        ),
        Err(StoreError::Conflict(
            StoreConflict::DuplicateToolCallId { .. }
        ))
    ));
    assert!(matches!(
        store.transition_tool(ToolCallTransition {
            chat: chat.clone(),
            turn_id: "turn-1".into(),
            tool_call_id: "tool-1".into(),
            next_state: ToolCallState::Streaming(ToolCallStreamingState {
                tool_call_id: "tool-1".into(),
                tool_name: "replace_string".into(),
                display_name: "Replace text".into(),
                intention: None,
                contributor: None,
                meta: None,
                partial_input: None,
                invocation_message: None,
            }),
            actions: vec![tool_start_action("turn-1", "tool-1")],
            origin: None,
            allow_command: None,
        }),
        Err(StoreError::Conflict(StoreConflict::InvalidToolState { .. }))
    ));
    assert!(store
        .append_text(&chat, "turn-1", "response-turn-1", "")
        .is_err());
    assert_eq!(store.server_sequence()?, after_tool_sequence);
    assert_ne!(before_sequence, after_tool_sequence);
    assert_ne!(
        serde_json::to_value(chat_state(store, chat.clone())?)?,
        before_state
    );
    assert_eq!(
        serde_json::to_value(chat_state(store, chat)?)?,
        after_tool_state
    );
    Ok(())
}

#[test]
fn persisted_catalogue_promotes_blocked_and_error_chats_over_a_latest_idle_default(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = TestStore::new()?;
    let store = fixture.store();

    let (blocked_session, idle_chat) = create_ready(store)?;
    let blocked_chat: Uri = format!("ahp-chat:/{}", Uuid::new_v4());
    store.create_chat(CreateChat {
        session: blocked_session.clone(),
        chat: blocked_chat.clone(),
    })?;
    store.start_turn(TurnStart {
        chat: blocked_chat.clone(),
        action: start("blocked-turn")?,
        origin: None,
        legacy_default_selection: Some(default_selection()),
    })?;
    store.start_tool(
        &blocked_chat,
        "blocked-turn",
        streaming_part("blocked-tool"),
        tool_start_action("blocked-turn", "blocked-tool"),
    )?;
    store.transition_tool(ToolCallTransition {
        chat: blocked_chat.clone(),
        turn_id: "blocked-turn".into(),
        tool_call_id: "blocked-tool".into(),
        next_state: pending_confirmation_state("blocked-tool")?,
        actions: vec![ready_action("blocked-turn", "blocked-tool")],
        origin: None,
        allow_command: None,
    })?;
    set_chat_modified_at(store, &blocked_chat, "2026-01-01T00:00:00.000Z")?;
    let latest_idle = chat_state(store, idle_chat.clone())?;
    let blocked = chat_state(store, blocked_chat.clone())?;
    assert_eq!(blocked.status, SessionStatus::InputNeeded.bits());
    assert_eq!(
        store.session_summary(&blocked_session)?.status,
        blocked.status
    );
    assert_eq!(
        store.session_summary(&blocked_session)?.modified_at,
        latest_idle.modified_at
    );
    assert_eq!(
        session_state(store, blocked_session.clone())?.status,
        SessionStatus::InputNeeded.bits()
    );
    replace_active_tool_state(
        store,
        &blocked_chat,
        "blocked-tool",
        pending_result_confirmation_state("blocked-tool")?,
    )?;
    assert_eq!(
        chat_state(store, blocked_chat)?.status,
        SessionStatus::InputNeeded.bits()
    );

    let (error_session, error_idle_chat) = create_ready(store)?;
    let error_chat: Uri = format!("ahp-chat:/{}", Uuid::new_v4());
    store.create_chat(CreateChat {
        session: error_session.clone(),
        chat: error_chat.clone(),
    })?;
    store.start_turn(TurnStart {
        chat: error_chat.clone(),
        action: start("error-turn")?,
        origin: None,
        legacy_default_selection: Some(default_selection()),
    })?;
    store.finish_turn(FinishTurn::Error {
        chat: error_chat.clone(),
        turn_id: "error-turn".into(),
        error: ErrorInfo {
            error_type: "test_error".into(),
            message: "test error".into(),
            stack: None,
            meta: None,
        },
    })?;
    set_chat_modified_at(store, &error_chat, "2026-01-01T00:00:00.000Z")?;
    let latest_idle = chat_state(store, error_idle_chat)?;
    let error = chat_state(store, error_chat)?;
    assert_eq!(error.status, SessionStatus::Error.bits());
    assert!(matches!(
        error
            .turns
            .last()
            .and_then(|turn| turn.response_parts.last()),
        Some(ResponsePart::Error(_))
    ));
    assert_eq!(store.session_summary(&error_session)?.status, error.status);
    assert_eq!(
        store.session_summary(&error_session)?.modified_at,
        latest_idle.modified_at
    );
    assert_eq!(
        session_state(store, error_session.clone())?.status,
        SessionStatus::Error.bits()
    );
    let listed = store.list_sessions()?;
    assert_eq!(
        listed
            .iter()
            .find(|summary| summary.resource == blocked_session)
            .map(|summary| summary.status),
        Some(SessionStatus::InputNeeded.bits())
    );
    assert_eq!(
        listed
            .iter()
            .find(|summary| summary.resource == error_session)
            .map(|summary| summary.status),
        Some(SessionStatus::Error.bits())
    );
    Ok(())
}

#[test]
fn replay_is_bounded_and_old_sequences_require_snapshots() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = TestStore::new()?;
    let store = fixture.store();
    for index in 0..=crate::tables::REPLAY_LIMIT {
        store.reject_action(
            "ahp-root://".into(),
            StateAction::ChatTurnStarted(start(&format!("rejected-{index}"))?),
            None,
            "test".into(),
        )?;
    }
    assert!(matches!(
        store.replay_since(0)?,
        ActionReplay::SnapshotRequired
    ));
    let ActionReplay::Replay(replay) = store.replay_since(1)? else {
        return Err(std::io::Error::other("expected retained replay").into());
    };
    assert_eq!(i64::try_from(replay.len())?, crate::tables::REPLAY_LIMIT);
    assert_sequences(&replay);
    Ok(())
}

#[test]
fn restart_recovers_active_turn_and_terminalizes_tools() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = TestStore::new()?;
    let (session, chat) = create_ready(fixture.store())?;
    fixture.store().start_turn(TurnStart {
        chat: chat.clone(),
        action: start("interrupted")?,
        origin: None,
        legacy_default_selection: Some(default_selection()),
    })?;
    fixture.store().start_tool(
        &chat,
        "interrupted",
        streaming_part("interrupted-tool"),
        tool_start_action("interrupted", "interrupted-tool"),
    )?;
    let before = fixture.store().server_sequence()?;
    fixture.reopen()?;
    let store = fixture.store();
    let state = chat_state(store, chat)?;
    assert!(state.active_turn.is_none());
    let turn = state
        .turns
        .last()
        .ok_or_else(|| std::io::Error::other("missing recovered turn"))?;
    assert_eq!(turn.state, TurnState::Error);
    let ResponsePart::ToolCall(tool) = &turn.response_parts[1] else {
        return Err(std::io::Error::other("missing recovered tool").into());
    };
    assert!(matches!(tool.tool_call, ToolCallState::Cancelled(_)));
    assert_eq!(store.server_sequence()?, before + 2);
    assert_eq!(
        store.session_summary(&session)?.status,
        ahp_types::state::SessionStatus::Error.bits()
    );
    let recovery_actions = journal(store)?;
    assert!(matches!(
        &recovery_actions[recovery_actions.len() - 2].action,
        StateAction::ChatError(_)
    ));
    assert!(matches!(
        recovery_actions.last().map(|item| &item.action),
        Some(StateAction::SessionChatUpdated(_))
    ));
    Ok(())
}

#[test]
fn exact_approvals_survive_reopen_without_honoring_legacy_grants(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = TestStore::new()?;
    fixture.store().test_write(|write| {
        write
            .open_table(META)
            .map_err(crate::db::backend)?
            .insert("shell_approved_commands", r#"["echo legacy"]"#)
            .map_err(crate::db::backend)?;
        Ok(())
    })?;
    assert!(!fixture.store().is_command_allowed(
        &ahp_server_core::workspace_agent_tools::ExactCommandApproval::new("echo legacy")?
    )?);
    let (_, chat) = create_ready(fixture.store())?;
    fixture.store().start_turn(TurnStart {
        chat: chat.clone(),
        action: start("approval-turn")?,
        origin: None,
        legacy_default_selection: Some(default_selection()),
    })?;
    fixture.store().start_tool(
        &chat,
        "approval-turn",
        streaming_part("approval-tool"),
        tool_start_action("approval-turn", "approval-tool"),
    )?;
    let approval = ahp_server_core::workspace_agent_tools::ExactCommandApproval::new("echo safe")?;
    fixture.store().transition_tool(ToolCallTransition {
        chat,
        turn_id: "approval-turn".into(),
        tool_call_id: "approval-tool".into(),
        next_state: running_state("approval-tool"),
        actions: vec![ready_action("approval-turn", "approval-tool")],
        origin: None,
        allow_command: Some(approval.clone()),
    })?;
    fixture.reopen()?;
    assert!(fixture.store().is_command_allowed(&approval)?);
    assert!(!fixture.store().is_command_allowed(
        &ahp_server_core::workspace_agent_tools::ExactCommandApproval::new("echo safe && whoami")?
    )?);
    fixture.store().reset_command_approvals()?;
    assert!(!fixture.store().is_command_allowed(&approval)?);
    Ok(())
}

#[test]
fn completed_turn_history_is_limited_to_256_entries() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = TestStore::new()?;
    let store = fixture.store();
    let (_, chat) = create_ready(store)?;
    for index in 0..257 {
        let turn_id = format!("turn-{index}");
        store.start_turn(TurnStart {
            chat: chat.clone(),
            action: start(&turn_id)?,
            origin: None,
            legacy_default_selection: Some(default_selection()),
        })?;
        store.finish_turn(FinishTurn::Complete {
            chat: chat.clone(),
            turn_id,
            model_cursor: None,
        })?;
    }
    let state = chat_state(store, chat)?;
    assert_eq!(state.turns.len(), 256);
    assert_eq!(state.turns[0].id, "turn-1");
    assert_eq!(state.turns[255].id, "turn-256");
    Ok(())
}

#[test]
fn legacy_json_without_response_parts_remains_readable() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = TestStore::new()?;
    let store = fixture.store();
    let session: Uri = "ahp-session:/legacy".into();
    let chat: Uri = "ahp-chat:/legacy".into();
    store.test_write(|write| {
        let session_json = json!({
            "uri": session,
            "provider": "openai",
            "title": "legacy",
            "created_at": "2026-01-01T00:00:00.000Z",
            "working_directories": ["file:///legacy"],
            "lifecycle": "Ready",
            "chats": [chat],
            "default_chat": chat
        })
        .to_string();
        let chat_json = json!({
            "uri": chat,
            "session_uri": session,
            "title": "legacy chat",
            "created_at": "2026-01-01T00:00:00.000Z",
            "modified_at": "2026-01-01T00:00:00.000Z",
            "turns": [{
                "id": "legacy-turn",
                "started_at": "2026-01-01T00:00:00.000Z",
                "duration": 1,
                "message": { "text": "hello", "origin": { "kind": "user" } },
                "response": "legacy answer",
                "state": "Complete",
                "error": null
            }],
            "active": null
        })
        .to_string();
        write
            .open_table(SESSIONS)
            .map_err(crate::db::backend)?
            .insert(session.as_str(), session_json.as_str())
            .map_err(crate::db::backend)?;
        write
            .open_table(CHATS)
            .map_err(crate::db::backend)?
            .insert(chat.as_str(), chat_json.as_str())
            .map_err(crate::db::backend)?;
        let ids = serde_json::to_string(&vec![session.clone()]).map_err(crate::db::json_error)?;
        write
            .open_table(META)
            .map_err(crate::db::backend)?
            .insert(SESSION_IDS, ids.as_str())
            .map_err(crate::db::backend)?;
        Ok(())
    })?;
    let state = chat_state(store, chat)?;
    let ResponsePart::Markdown(markdown) = &state.turns[0].response_parts[0] else {
        return Err(std::io::Error::other("legacy markdown was not synthesized").into());
    };
    assert_eq!(markdown.content, "legacy answer");
    Ok(())
}

#[test]
fn failed_precommit_work_rolls_back_state_and_journal() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = TestStore::new()?;
    let store = fixture.store();
    let (_, chat) = create_ready(store)?;
    let before = serde_json::to_value(chat_state(store, chat.clone())?)?;
    let sequence = store.server_sequence()?;
    let result: Result<(), StoreError> = store.test_write(|write| {
        let changed = json!({
            "uri": chat,
            "session_uri": "ahp-session:/ignored",
            "title": "must not commit",
            "created_at": "2026-01-01T00:00:00.000Z",
            "modified_at": "2026-01-01T00:00:00.000Z",
            "turns": [],
            "active": null
        })
        .to_string();
        write
            .open_table(CHATS)
            .map_err(crate::db::backend)?
            .insert(chat.as_str(), changed.as_str())
            .map_err(crate::db::backend)?;
        Err(StoreError::Backend("injected precommit failure".into()))
    });
    assert!(result.is_err());
    assert_eq!(store.server_sequence()?, sequence);
    assert_eq!(serde_json::to_value(chat_state(store, chat)?)?, before);
    Ok(())
}
