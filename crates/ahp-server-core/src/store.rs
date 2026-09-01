//! Durable-state port and crash-atomic transition inputs.

use ahp_types::actions::{ActionEnvelope, ActionOrigin, ChatTurnStartedAction, StateAction};
use ahp_types::common::Uri;
use ahp_types::state::{
    ChatSummary, ErrorInfo, ModelSelection, ResponsePart, SessionSummary, Snapshot, ToolCallState,
};
use thiserror::Error;
use workspace_agent_tools::ExactCommandApproval;

use crate::ModelCatalog;

/// A replay query result.
#[derive(Clone, Debug, PartialEq)]
pub enum ActionReplay {
    /// Every action after the requested sequence remains available.
    Replay(Vec<ActionEnvelope>),
    /// The requested sequence fell outside the durable replay window.
    SnapshotRequired,
}

/// A transaction that has committed its state and replay journal together.
#[derive(Clone, Debug, PartialEq)]
pub struct CommittedTransition<T> {
    /// Ordered action envelopes assigned durable, monotonic `serverSeq` values.
    pub envelopes: Vec<ActionEnvelope>,
    /// The typed output needed for the next core operation.
    pub value: T,
}

/// A durable-store failure that was not a lifecycle conflict.
#[derive(Debug, Error)]
pub enum StoreError {
    /// Persistence implementation failure.
    #[error("host store failure: {0}")]
    Backend(String),
    /// A semantic conflict prevents the requested atomic transition.
    #[error(transparent)]
    Conflict(#[from] StoreConflict),
}

/// Explicit invalid or unexpected lifecycle state.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StoreConflict {
    /// A requested state-bearing channel does not exist.
    #[error("unknown channel {0:?}")]
    UnknownChannel(Uri),
    /// A session creation URI already exists.
    #[error("session {0:?} already exists")]
    SessionExists(Uri),
    /// A chat creation URI already exists.
    #[error("chat {0:?} already exists")]
    ChatExists(Uri),
    /// The session has not reached ready state.
    #[error("session {0:?} is not ready")]
    SessionNotReady(Uri),
    /// A chat already has an active turn.
    #[error("chat {0:?} already has an active turn")]
    ActiveTurn(Uri),
    /// The chat has no active turn.
    #[error("chat {0:?} has no active turn")]
    NoActiveTurn(Uri),
    /// The requested turn is not the active one.
    #[error("turn {turn:?} is not active in chat {chat:?}")]
    TurnMismatch {
        /// Chat channel.
        chat: Uri,
        /// Turn identifier.
        turn: String,
    },
    /// A model reused a previously durable tool-call identifier.
    #[error("duplicate tool call id {tool_call_id:?} in chat {chat:?}")]
    DuplicateToolCallId {
        /// Chat channel.
        chat: Uri,
        /// Tool identifier.
        tool_call_id: String,
    },
    /// The requested tool call is absent from the active turn.
    #[error("tool call {tool_call_id:?} is absent from chat {chat:?}")]
    ToolNotFound {
        /// Chat channel.
        chat: Uri,
        /// Tool identifier.
        tool_call_id: String,
    },
    /// A requested server-tool state edge is invalid.
    #[error("invalid transition for tool call {tool_call_id:?} in chat {chat:?}")]
    InvalidToolState {
        /// Chat channel.
        chat: Uri,
        /// Tool identifier.
        tool_call_id: String,
    },
    /// No confirmation waiter is registered for the tool.
    #[error("no pending confirmation for tool call {tool_call_id:?} in chat {chat:?}")]
    NoPendingApproval {
        /// Chat channel.
        chat: Uri,
        /// Tool identifier.
        tool_call_id: String,
    },
}

/// Parameters used to project one typed AHP snapshot.
#[derive(Clone, Debug)]
pub struct SnapshotRequest {
    /// Requested state-bearing channel.
    pub channel: Uri,
    /// Models and configuration schema shown in root and session state.
    pub model_catalog: ModelCatalog,
    /// Server tools to expose in session state.
    pub server_tools: Vec<ahp_types::state::ToolDefinition>,
}

/// Input for durable session creation.
#[derive(Clone, Debug)]
pub struct CreateSession {
    /// New session channel.
    pub session: Uri,
    /// New default chat channel.
    pub default_chat: Uri,
    /// Provider name.
    pub provider: String,
    /// Selected workspace URI.
    pub workspace: Uri,
    /// Resolved default model used for every turn without a message override.
    pub model_selection: ModelSelection,
}

/// Result of creating a session and default chat.
#[derive(Clone, Debug)]
pub struct CreatedSession {
    /// Created session channel.
    pub session: Uri,
    /// Created default chat summary.
    pub default_chat: ChatSummary,
    /// Selected workspace persisted for future turn execution.
    pub workspace: Uri,
}

/// Input for creating an empty chat.
#[derive(Clone, Debug)]
pub struct CreateChat {
    /// Owning session channel.
    pub session: Uri,
    /// New chat channel.
    pub chat: Uri,
}

/// Input for setting a newly created session ready.
#[derive(Clone, Debug)]
pub struct MarkSessionReady {
    /// Session channel.
    pub session: Uri,
}

/// Result of deleting a session.
#[derive(Clone, Debug)]
pub struct DisposedSession {
    /// Deleted session channel.
    pub session: Uri,
    /// Deleted chat channels, used to cancel in-memory work and subscriptions.
    pub chats: Vec<Uri>,
}

/// Input for beginning an active chat turn.
#[derive(Clone, Debug)]
pub struct TurnStart {
    /// Chat channel.
    pub chat: Uri,
    /// Validated client action.
    pub action: ChatTurnStartedAction,
    /// Originating client sequence, if supplied.
    pub origin: Option<ActionOrigin>,
    /// Default used only when projecting a legacy session with no saved selection.
    pub legacy_default_selection: ModelSelection,
}

/// Durable output required to execute a started turn.
#[derive(Clone, Debug)]
pub struct StartedTurn {
    /// Chat channel.
    pub chat: Uri,
    /// Active turn identifier.
    pub turn_id: String,
    /// Original user message.
    pub message: ahp_types::state::Message,
    /// Persisted session workspace.
    pub workspace: Uri,
    /// The validated model selection that was durably recorded on the message.
    pub model_selection: ModelSelection,
}

/// A fully validated mutable session-model change.
#[derive(Clone, Debug)]
pub struct UpdateSessionConfig {
    /// Session channel whose default selection changes.
    pub session: Uri,
    /// Canonical selection persisted for future turns.
    pub model_selection: ModelSelection,
    /// Originating client sequence, if supplied.
    pub origin: Option<ActionOrigin>,
}

/// The terminal state a store must apply atomically to an active turn.
#[derive(Clone, Debug)]
pub enum FinishTurn {
    /// A turn completed naturally.
    Complete {
        /// Chat channel.
        chat: Uri,
        /// Active turn.
        turn_id: String,
    },
    /// A client cancelled a turn.
    Cancelled {
        /// Chat channel.
        chat: Uri,
        /// Active turn.
        turn_id: String,
        /// Originating client sequence.
        origin: Option<ActionOrigin>,
    },
    /// Model, tool, or store orchestration failed.
    Error {
        /// Chat channel.
        chat: Uri,
        /// Active turn.
        turn_id: String,
        /// Stable error information.
        error: ErrorInfo,
    },
}

/// A server-tool state change already validated by core.
#[derive(Clone, Debug)]
pub struct ToolCallTransition {
    /// Chat channel.
    pub chat: Uri,
    /// Active turn identifier.
    pub turn_id: String,
    /// Model-issued unique tool-call identifier.
    pub tool_call_id: String,
    /// State which replaces the current tool-call state.
    pub next_state: ToolCallState,
    /// Actions recorded with this state update.
    pub actions: Vec<StateAction>,
    /// Action origin, when this was triggered by a client confirmation.
    pub origin: Option<ActionOrigin>,
    /// Exact command approval added in this same transaction, if any.
    pub allow_command: Option<ExactCommandApproval>,
}

/// Requested transition phase, useful to durable adapters and test instrumentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolPhase {
    /// Model has begun a tool call.
    Started,
    /// More raw JSON arguments arrived.
    Streaming,
    /// Input is ready or waits for confirmation.
    Ready,
    /// Tool execution reached a terminal state.
    Completed,
}

/// Identifies an in-memory confirmation waiter.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ApprovalKey {
    /// Chat channel.
    pub chat: Uri,
    /// Turn identifier.
    pub turn_id: String,
    /// Tool-call identifier.
    pub tool_call_id: String,
}

/// Persistence boundary for an AHP host.
///
/// Each `*_transition` operation must update its state and append all resulting
/// `ActionEnvelope`s in one crash-atomic commit. Returning `Ok` means the
/// envelopes are safe to fan out; core never publishes speculative actions.
pub trait HostStore: Send + Sync {
    /// Current durable server sequence.
    fn server_sequence(&self) -> Result<i64, StoreError>;
    /// Gets a typed root, session, or chat snapshot.
    fn snapshot(&self, request: SnapshotRequest) -> Result<Snapshot, StoreError>;
    /// Reads a durable replay range.
    fn replay_since(&self, sequence: i64) -> Result<ActionReplay, StoreError>;
    /// Lists root session summaries.
    fn list_sessions(&self) -> Result<Vec<SessionSummary>, StoreError>;
    /// Gets one root session summary.
    fn session_summary(&self, session: &str) -> Result<SessionSummary, StoreError>;
    /// Atomically creates session state and its initial action journal entries.
    fn create_session(
        &self,
        request: CreateSession,
    ) -> Result<CommittedTransition<CreatedSession>, StoreError>;
    /// Atomically persists a session model selection and its authoritative action.
    fn update_session_config(
        &self,
        request: UpdateSessionConfig,
    ) -> Result<CommittedTransition<()>, StoreError>;
    /// Atomically transitions a session to ready.
    fn mark_session_ready(
        &self,
        request: MarkSessionReady,
    ) -> Result<CommittedTransition<()>, StoreError>;
    /// Atomically creates a child chat.
    fn create_chat(
        &self,
        request: CreateChat,
    ) -> Result<CommittedTransition<ChatSummary>, StoreError>;
    /// Atomically deletes a session and its chats.
    fn dispose_session(
        &self,
        session: &str,
    ) -> Result<CommittedTransition<DisposedSession>, StoreError>;
    /// Atomically records an active turn and initial markdown response part.
    fn start_turn(
        &self,
        request: TurnStart,
    ) -> Result<CommittedTransition<StartedTurn>, StoreError>;
    /// Atomically appends a text chunk and its `chat/delta` action.
    fn append_text(
        &self,
        chat: &str,
        turn_id: &str,
        chunk: &str,
    ) -> Result<CommittedTransition<()>, StoreError>;
    /// Atomically records a streaming tool response part and start action.
    fn start_tool(
        &self,
        chat: &str,
        turn_id: &str,
        part: ResponsePart,
        action: StateAction,
    ) -> Result<CommittedTransition<()>, StoreError>;
    /// Atomically appends raw tool-input content and its delta action.
    fn append_tool_delta(
        &self,
        chat: &str,
        turn_id: &str,
        tool_call_id: &str,
        delta: &str,
        action: StateAction,
    ) -> Result<CommittedTransition<()>, StoreError>;
    /// Atomically replaces a tool state, its actions, and optional exact approval.
    fn transition_tool(
        &self,
        transition: ToolCallTransition,
    ) -> Result<CommittedTransition<()>, StoreError>;
    /// Atomically terminalizes an active turn and journals terminal actions.
    fn finish_turn(&self, request: FinishTurn) -> Result<CommittedTransition<i64>, StoreError>;
    /// Atomically journals a rejected client action.
    fn reject_action(
        &self,
        channel: Uri,
        action: StateAction,
        origin: Option<ActionOrigin>,
        reason: String,
    ) -> Result<CommittedTransition<()>, StoreError>;
    /// Looks up a persisted exact-command approval.
    fn is_command_allowed(&self, approval: &ExactCommandApproval) -> Result<bool, StoreError>;
    /// Removes every exact-command approval.
    fn reset_command_approvals(&self) -> Result<(), StoreError>;
}
