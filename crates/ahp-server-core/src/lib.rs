//! Transport-independent Agent Host Protocol server lifecycle.

#![forbid(unsafe_code)]

mod config;
mod host;
mod memory;
mod model_catalog;
mod presentation;
mod profile;
mod rpc;
mod store;
mod turns;

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
