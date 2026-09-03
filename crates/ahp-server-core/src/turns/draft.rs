//! Validation and crash-atomic persistence of client chat drafts.

use ahp_types::actions::{ChatDraftChangedAction, StateAction};
use ahp_types::state::MessageKind;
use serde_json::Value;

use super::reject;
use crate::host::CoreHost;
use crate::store::{ChangeDraft, DraftTurnPrecondition};

const EXPECTED_TURN_ID_META: &str = "com.microsoft.agent-host-protocol-samples.expectedTurnId";

pub(crate) async fn change_draft(
    host: &CoreHost,
    channel: &str,
    raw: Value,
    action: StateAction,
    origin: Option<ahp_types::actions::ActionOrigin>,
) {
    let changed = match serde_json::from_value::<ChatDraftChangedAction>(raw) {
        Ok(changed) => changed,
        Err(error) => {
            reject(
                host,
                channel.into(),
                action,
                origin,
                format!("invalid chat/draftChanged: {error}"),
            )
            .await;
            return;
        }
    };
    let mut draft = changed.draft;
    if let Some(draft) = &draft {
        if !matches!(draft.origin.kind, MessageKind::User) {
            reject(
                host,
                channel.into(),
                action,
                origin,
                "only user messages may be chat drafts",
            )
            .await;
            return;
        }
        if let Some(selection) = &draft.model {
            if let Err(error) = host.inner.model_catalog.validate_selection(selection) {
                reject(host, channel.into(), action, origin, error).await;
                return;
            }
        }
    }
    let turn_precondition = match take_turn_precondition(&mut draft) {
        Ok(precondition) => precondition,
        Err(message) => {
            reject(host, channel.into(), action, origin, message).await;
            return;
        }
    };
    let _guard = host.inner.publish_lock.lock().await;
    match host.inner.store.change_draft(ChangeDraft {
        chat: channel.into(),
        draft,
        turn_precondition,
        origin: origin.clone(),
    }) {
        Ok(transition) => host.fanout(&transition).await,
        Err(error) => {
            drop(_guard);
            reject(host, channel.into(), action, origin, error).await;
        }
    }
}

fn take_turn_precondition(
    draft: &mut Option<ahp_types::state::Message>,
) -> Result<DraftTurnPrecondition, &'static str> {
    let Some(draft) = draft.as_mut() else {
        return Ok(DraftTurnPrecondition::Unchecked);
    };
    let Some(meta) = draft.meta.as_mut() else {
        return Ok(DraftTurnPrecondition::Unchecked);
    };
    let Some(value) = meta.remove(EXPECTED_TURN_ID_META) else {
        return Ok(DraftTurnPrecondition::Unchecked);
    };
    if meta.is_empty() {
        draft.meta = None;
    }
    match value {
        Value::Null => Ok(DraftTurnPrecondition::Latest(None)),
        Value::String(turn_id) if !turn_id.is_empty() => {
            Ok(DraftTurnPrecondition::Latest(Some(turn_id)))
        }
        _ => Err("draft expected-turn metadata must be a non-empty string or null"),
    }
}
