//! Durable [`HostStore`](ahp_server_core::HostStore) backed by `redb`.
//!
//! State changes and their replay envelopes are committed in one redb write
//! transaction. A successful transition can therefore be published safely.

#![forbid(unsafe_code)]

mod approvals;
mod db;
mod queries;
mod recovery;
mod replay;
mod sessions;
mod tables;
mod transition;
mod types;

use std::path::Path;
use std::sync::Mutex;

use crate::db::backend;
use crate::replay::{Draft, TransitionWork};
use ahp_server_core::{
    ActionReplay, CommittedTransition, CreateChat, CreateSession, CreatedSession, DisposedSession,
    FinishTurn, HostStore, MarkSessionReady, SnapshotRequest, StartedTurn, StoreConflict,
    StoreError, ToolCallTransition, TurnStart,
};
use ahp_types::actions::{ActionOrigin, StateAction};
use ahp_types::common::{Uri, ROOT_RESOURCE_URI};

/// A redb-backed implementation of [`HostStore`].
pub struct RedbHostStore {
    database: Mutex<Option<redb::Database>>,
}

/// Constructor failures and `HostStore` operation failures.
pub type RedbStoreError = StoreError;

impl RedbHostStore {
    /// Opens or creates the database at `path` and recovers interrupted turns.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RedbStoreError> {
        Self::open_database(path.as_ref())
    }

    /// Releases the database handle early, primarily for Windows file cleanup.
    ///
    /// Calling this is optional: dropping the store releases the same handle.
    pub fn close(&self) -> Result<(), StoreError> {
        self.close_database()
    }
}

impl HostStore for RedbHostStore {
    fn server_sequence(&self) -> Result<i64, StoreError> {
        let database = self.database()?;
        let read = database.get()?.begin_read().map_err(backend)?;
        let meta = read.open_table(tables::META).map_err(backend)?;
        replay::meta_value_i64_read(&meta, tables::SERVER_SEQUENCE)
    }

    fn snapshot(&self, request: SnapshotRequest) -> Result<ahp_types::state::Snapshot, StoreError> {
        self.snapshot_for(request)
    }

    fn replay_since(&self, sequence: i64) -> Result<ActionReplay, StoreError> {
        self.replay_since_sequence(sequence)
    }

    fn list_sessions(&self) -> Result<Vec<ahp_types::state::SessionSummary>, StoreError> {
        self.session_summaries()
    }

    fn session_summary(
        &self,
        session: &str,
    ) -> Result<ahp_types::state::SessionSummary, StoreError> {
        self.summary_for(session)
    }

    fn create_session(
        &self,
        request: CreateSession,
    ) -> Result<CommittedTransition<CreatedSession>, StoreError> {
        self.create_session_transition(request)
    }

    fn mark_session_ready(
        &self,
        request: MarkSessionReady,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.mark_session_ready_transition(request)
    }

    fn create_chat(
        &self,
        request: CreateChat,
    ) -> Result<CommittedTransition<ahp_types::state::ChatSummary>, StoreError> {
        self.create_chat_transition(request)
    }

    fn dispose_session(
        &self,
        session: &str,
    ) -> Result<CommittedTransition<DisposedSession>, StoreError> {
        self.dispose_session_transition(session)
    }

    fn start_turn(
        &self,
        request: TurnStart,
    ) -> Result<CommittedTransition<StartedTurn>, StoreError> {
        self.start_turn_transition(request)
    }

    fn append_text(
        &self,
        chat: &str,
        turn_id: &str,
        chunk: &str,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.append_text_transition(chat, turn_id, chunk)
    }

    fn start_tool(
        &self,
        chat: &str,
        turn_id: &str,
        part: ahp_types::state::ResponsePart,
        action: StateAction,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.start_tool_transition(chat, turn_id, part, action)
    }

    fn append_tool_delta(
        &self,
        chat: &str,
        turn_id: &str,
        tool_call_id: &str,
        delta: &str,
        action: StateAction,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.append_tool_delta_transition(chat, turn_id, tool_call_id, delta, action)
    }

    fn transition_tool(
        &self,
        transition: ToolCallTransition,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.transition_tool_transition(transition)
    }

    fn finish_turn(&self, request: FinishTurn) -> Result<CommittedTransition<i64>, StoreError> {
        self.finish_turn_transition(request)
    }

    fn reject_action(
        &self,
        channel: Uri,
        action: StateAction,
        origin: Option<ActionOrigin>,
        reason: String,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.commit_transition(move |write| {
            let known = channel == ROOT_RESOURCE_URI
                || sessions::load_session(write, &channel)?.is_some()
                || sessions::load_chat(write, &channel)?.is_some();
            if !known {
                return Err(StoreConflict::UnknownChannel(channel).into());
            }
            Ok(TransitionWork::new(
                (),
                vec![Draft::rejection(channel, action, origin, reason)],
            ))
        })
    }

    fn is_command_allowed(
        &self,
        approval: &ahp_server_core::workspace_agent_tools::ExactCommandApproval,
    ) -> Result<bool, StoreError> {
        self.command_is_allowed(approval)
    }

    fn reset_command_approvals(&self) -> Result<(), StoreError> {
        self.reset_exact_command_approvals()
    }
}

#[cfg(test)]
mod tests;
