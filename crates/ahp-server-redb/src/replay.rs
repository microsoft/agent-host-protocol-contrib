//! Crash-atomic action-journal allocation and replay reads.

use ahp_server_core::{ActionReplay, CommittedTransition, StoreError};
use ahp_types::actions::{ActionEnvelope, ActionOrigin, StateAction};
use ahp_types::common::Uri;
use redb::ReadableTable;

use crate::db::{backend, json_error};
use crate::tables::{ACTIONS, FIRST_ACTION_SEQUENCE, META, REPLAY_LIMIT, SERVER_SEQUENCE};
use crate::RedbHostStore;

pub(crate) struct Draft {
    channel: Uri,
    action: StateAction,
    origin: Option<ActionOrigin>,
    rejection_reason: Option<String>,
}

impl Draft {
    pub(crate) fn action(
        channel: impl Into<Uri>,
        action: StateAction,
        origin: Option<ActionOrigin>,
    ) -> Self {
        Self {
            channel: channel.into(),
            action,
            origin,
            rejection_reason: None,
        }
    }

    pub(crate) fn rejection(
        channel: Uri,
        action: StateAction,
        origin: Option<ActionOrigin>,
        rejection_reason: String,
    ) -> Self {
        Self {
            channel,
            action,
            origin,
            rejection_reason: Some(rejection_reason),
        }
    }
}

pub(crate) struct TransitionWork<T> {
    value: T,
    drafts: Vec<Draft>,
}

impl<T> TransitionWork<T> {
    pub(crate) fn new(value: T, drafts: Vec<Draft>) -> Self {
        Self { value, drafts }
    }
}

impl RedbHostStore {
    pub(crate) fn commit_transition<T>(
        &self,
        operation: impl FnOnce(&redb::WriteTransaction) -> Result<TransitionWork<T>, StoreError>,
    ) -> Result<CommittedTransition<T>, StoreError> {
        let database = self.database()?;
        let write = database.get()?.begin_write().map_err(backend)?;
        let work = operation(&write)?;
        if work.drafts.is_empty() {
            return Err(StoreError::Backend(
                "state transition must contain an action envelope".into(),
            ));
        }
        let envelopes = append(&write, work.drafts)?;
        write.commit().map_err(backend)?;
        Ok(CommittedTransition {
            envelopes,
            value: work.value,
        })
    }

    pub(crate) fn replay_since_sequence(&self, sequence: i64) -> Result<ActionReplay, StoreError> {
        let database = self.database()?;
        let read = database.get()?.begin_read().map_err(backend)?;
        let meta = read.open_table(META).map_err(backend)?;
        let first = meta_value_i64_read(&meta, FIRST_ACTION_SEQUENCE)?;
        let current = meta_value_i64_read(&meta, SERVER_SEQUENCE)?;
        drop(meta);
        if sequence < first.saturating_sub(1) {
            return Ok(ActionReplay::SnapshotRequired);
        }
        let start = sequence.saturating_add(1).max(1);
        if start > current {
            return Ok(ActionReplay::Replay(Vec::new()));
        }
        let actions = read.open_table(ACTIONS).map_err(backend)?;
        let mut replay = Vec::new();
        for action_sequence in start..=current {
            let value = actions
                .get(action_sequence)
                .map_err(backend)?
                .ok_or_else(|| {
                    StoreError::Backend(format!(
                        "action journal is missing contiguous sequence {action_sequence}"
                    ))
                })?;
            replay.push(serde_json::from_str(value.value()).map_err(json_error)?);
        }
        drop(actions);
        Ok(ActionReplay::Replay(replay))
    }
}

fn append(
    write: &redb::WriteTransaction,
    drafts: Vec<Draft>,
) -> Result<Vec<ActionEnvelope>, StoreError> {
    let meta = write.open_table(META).map_err(backend)?;
    let current = meta_value_i64(&meta, SERVER_SEQUENCE)?;
    let first = meta_value_i64(&meta, FIRST_ACTION_SEQUENCE)?;
    drop(meta);
    let count = i64::try_from(drafts.len())
        .map_err(|error| StoreError::Backend(format!("too many transition actions: {error}")))?;
    let last = current
        .checked_add(count)
        .ok_or_else(|| StoreError::Backend("server sequence overflow".into()))?;
    let envelopes = drafts
        .into_iter()
        .enumerate()
        .map(|(index, draft)| {
            let offset = i64::try_from(index)
                .map_err(|error| StoreError::Backend(format!("action index overflow: {error}")))?;
            let sequence = current
                .checked_add(offset)
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| StoreError::Backend("server sequence overflow".into()))?;
            Ok(ActionEnvelope {
                channel: draft.channel,
                action: draft.action,
                server_seq: u64::try_from(sequence)
                    .map_err(|error| StoreError::Backend(error.to_string()))?,
                origin: draft.origin,
                rejection_reason: draft.rejection_reason,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    let retained = last.saturating_sub(first).saturating_add(1);
    let next_first = first.saturating_add(retained.saturating_sub(REPLAY_LIMIT));

    let mut actions = write.open_table(ACTIONS).map_err(backend)?;
    for envelope in &envelopes {
        let sequence = i64::try_from(envelope.server_seq)
            .map_err(|error| StoreError::Backend(error.to_string()))?;
        let serialized = serde_json::to_string(envelope).map_err(json_error)?;
        actions
            .insert(sequence, serialized.as_str())
            .map_err(backend)?;
    }
    for sequence in first..next_first {
        actions.remove(sequence).map_err(backend)?;
    }
    drop(actions);

    let mut meta = write.open_table(META).map_err(backend)?;
    meta.insert(SERVER_SEQUENCE, last.to_string().as_str())
        .map_err(backend)?;
    meta.insert(FIRST_ACTION_SEQUENCE, next_first.to_string().as_str())
        .map_err(backend)?;
    drop(meta);
    Ok(envelopes)
}

pub(crate) fn meta_value_i64(meta: &redb::Table<&str, &str>, key: &str) -> Result<i64, StoreError> {
    let value = meta
        .get(key)
        .map_err(backend)?
        .ok_or_else(|| StoreError::Backend(format!("{key} metadata is missing")))?;
    value
        .value()
        .parse()
        .map_err(|error| StoreError::Backend(format!("invalid {key} metadata: {error}")))
}

pub(crate) fn meta_value_i64_read(
    meta: &redb::ReadOnlyTable<&str, &str>,
    key: &str,
) -> Result<i64, StoreError> {
    let value = meta
        .get(key)
        .map_err(backend)?
        .ok_or_else(|| StoreError::Backend(format!("{key} metadata is missing")))?;
    value
        .value()
        .parse()
        .map_err(|error| StoreError::Backend(format!("invalid {key} metadata: {error}")))
}
