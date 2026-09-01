//! Complete in-memory [`HostStore`] implementation for embedding and tests.

mod lifecycle;
mod state;
mod transitions;

use std::sync::Mutex;

use ahp_types::actions::{ActionOrigin, StateAction};
use ahp_types::common::ROOT_RESOURCE_URI;
use workspace_agent_tools::ExactCommandApproval;

use self::state::Data;
use crate::store::{
    ActionReplay, CommittedTransition, CreateChat, CreateSession, CreatedSession, DisposedSession,
    FinishTurn, HostStore, MarkSessionReady, SnapshotRequest, StartedTurn, StoreConflict,
    StoreError, ToolCallTransition, TurnStart,
};

const REPLAY_LIMIT: usize = 2_048;

/// In-memory host store with the same atomic semantics as a durable adapter.
pub struct MemoryHostStore {
    data: Mutex<Data>,
}

impl Default for MemoryHostStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryHostStore {
    /// Creates an empty host store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Data::default()),
        }
    }

    fn transact<T>(
        &self,
        operation: impl FnOnce(&mut Data) -> Result<(T, Vec<Draft>), StoreError>,
    ) -> Result<CommittedTransition<T>, StoreError> {
        let mut data = self
            .data
            .lock()
            .map_err(|_| StoreError::Backend("memory store mutex is poisoned".into()))?;
        let mut candidate = data.clone();
        let (value, drafts) = operation(&mut candidate)?;
        if drafts.is_empty() {
            return Err(StoreError::Backend(
                "state transition must contain an action envelope".into(),
            ));
        }
        let envelopes = candidate.commit(drafts)?;
        *data = candidate;
        Ok(CommittedTransition { envelopes, value })
    }

    fn read<T>(
        &self,
        operation: impl FnOnce(&Data) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let data = self
            .data
            .lock()
            .map_err(|_| StoreError::Backend("memory store mutex is poisoned".into()))?;
        operation(&data)
    }
}

impl HostStore for MemoryHostStore {
    fn server_sequence(&self) -> Result<i64, StoreError> {
        self.read(|data| {
            i64::try_from(data.server_seq)
                .map_err(|_| StoreError::Backend("server sequence overflow".into()))
        })
    }

    fn snapshot(&self, request: SnapshotRequest) -> Result<ahp_types::state::Snapshot, StoreError> {
        self.read(|data| data.snapshot(request))
    }

    fn replay_since(&self, sequence: i64) -> Result<ActionReplay, StoreError> {
        self.read(|data| {
            if sequence < data.first_sequence.saturating_sub(1) {
                return Ok(ActionReplay::SnapshotRequired);
            }
            Ok(ActionReplay::Replay(
                data.replay
                    .iter()
                    .filter(|item| {
                        i64::try_from(item.server_seq)
                            .is_ok_and(|item_sequence| item_sequence > sequence)
                    })
                    .cloned()
                    .collect(),
            ))
        })
    }

    fn list_sessions(&self) -> Result<Vec<ahp_types::state::SessionSummary>, StoreError> {
        self.read(|data| {
            let mut summaries = data
                .sessions
                .values()
                .map(|session| data.session_summary(session))
                .collect::<Vec<_>>();
            summaries.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
            Ok(summaries)
        })
    }

    fn session_summary(
        &self,
        session: &str,
    ) -> Result<ahp_types::state::SessionSummary, StoreError> {
        self.read(|data| Ok(data.session_summary(data.session(session)?)))
    }

    fn create_session(
        &self,
        request: CreateSession,
    ) -> Result<CommittedTransition<CreatedSession>, StoreError> {
        lifecycle::create_session(self, request)
    }

    fn mark_session_ready(
        &self,
        request: MarkSessionReady,
    ) -> Result<CommittedTransition<()>, StoreError> {
        lifecycle::mark_session_ready(self, request)
    }

    fn create_chat(
        &self,
        request: CreateChat,
    ) -> Result<CommittedTransition<ahp_types::state::ChatSummary>, StoreError> {
        lifecycle::create_chat(self, request)
    }

    fn dispose_session(
        &self,
        session: &str,
    ) -> Result<CommittedTransition<DisposedSession>, StoreError> {
        lifecycle::dispose_session(self, session)
    }

    fn start_turn(
        &self,
        request: TurnStart,
    ) -> Result<CommittedTransition<StartedTurn>, StoreError> {
        transitions::start_turn(self, request)
    }

    fn append_text(
        &self,
        chat: &str,
        turn_id: &str,
        chunk: &str,
    ) -> Result<CommittedTransition<()>, StoreError> {
        transitions::append_text(self, chat, turn_id, chunk)
    }

    fn start_tool(
        &self,
        chat: &str,
        turn_id: &str,
        part: ahp_types::state::ResponsePart,
        action: StateAction,
    ) -> Result<CommittedTransition<()>, StoreError> {
        transitions::start_tool(self, chat, turn_id, part, action)
    }

    fn append_tool_delta(
        &self,
        chat: &str,
        turn_id: &str,
        tool_call_id: &str,
        delta: &str,
        action: StateAction,
    ) -> Result<CommittedTransition<()>, StoreError> {
        transitions::append_tool_delta(self, chat, turn_id, tool_call_id, delta, action)
    }

    fn transition_tool(
        &self,
        transition: ToolCallTransition,
    ) -> Result<CommittedTransition<()>, StoreError> {
        transitions::transition_tool(self, transition)
    }

    fn finish_turn(&self, request: FinishTurn) -> Result<CommittedTransition<i64>, StoreError> {
        self.transact(move |data| data.finish_turn(request))
    }

    fn reject_action(
        &self,
        channel: String,
        action: StateAction,
        origin: Option<ActionOrigin>,
        reason: String,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.transact(move |data| {
            if channel != ROOT_RESOURCE_URI
                && !data.sessions.contains_key(&channel)
                && !data.chats.contains_key(&channel)
            {
                return Err(StoreConflict::UnknownChannel(channel).into());
            }
            Ok(((), vec![Draft::rejection(channel, action, origin, reason)]))
        })
    }

    fn is_command_allowed(&self, approval: &ExactCommandApproval) -> Result<bool, StoreError> {
        self.read(|data| Ok(data.approvals.contains(approval)))
    }

    fn reset_command_approvals(&self) -> Result<(), StoreError> {
        self.data
            .lock()
            .map_err(|_| StoreError::Backend("memory store mutex is poisoned".into()))?
            .approvals
            .clear();
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Draft {
    pub(super) channel: String,
    pub(super) action: StateAction,
    pub(super) origin: Option<ActionOrigin>,
    pub(super) rejection_reason: Option<String>,
}

impl Draft {
    pub(super) fn action(
        channel: String,
        action: StateAction,
        origin: Option<ActionOrigin>,
    ) -> Self {
        Self {
            channel,
            action,
            origin,
            rejection_reason: None,
        }
    }

    fn rejection(
        channel: String,
        action: StateAction,
        origin: Option<ActionOrigin>,
        reason: String,
    ) -> Self {
        Self {
            channel,
            action,
            origin,
            rejection_reason: Some(reason),
        }
    }
}
