//! Transport-independent Agent Host Protocol server lifecycle.

#![forbid(unsafe_code)]

mod attachments;
pub mod catalog;
mod chat_state;
mod client_tools;
mod config;
mod host;
mod memory;
mod model_catalog;
mod presentation;
mod profile;
mod resources;
mod rpc;
mod store;
mod turns;

pub use attachments::materialize_attachments;
pub use client_tools::{client_tools, ChangeClientTool, ClientTool, ClientToolMutation};
pub use store::{ChangeSession, SessionMutation};

pub use chat_state::{ChangeChat, ChatMutation, PendingMessages};
pub use config::{CoreConfig, CoreConfigError};
pub use host::{CoreHost, CoreHostError, CorePeer, Shutdown};
pub use memory::MemoryHostStore;
pub use model_catalog::{ModelCatalog, ModelCatalogError};
pub use profile::{AgentProfile, ProfileError, ProfileFuture, WorkspaceAgentProfile};
pub use store::{
    ActionReplay, ApprovalKey, ChangeDraft, CommittedTransition, CreateChat, CreateSession,
    CreatedSession, DisposedSession, DraftTurnPrecondition, FinishTurn, HostStore,
    MarkSessionReady, ModelCursor, SnapshotRequest, StartedTurn, StoreConflict, StoreError,
    ToolCallTransition, ToolPhase, TurnStart, UpdateSessionConfig,
};

pub use ahp_types;
pub use openai_agent_model;
pub use workspace_agent_tools;
