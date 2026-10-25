//! Validation and dispatch of client lifecycle actions.

use ahp_types::actions::{ChatTurnStartedAction, SessionConfigChangedAction, StateAction};
use ahp_types::state::MessageKind;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{confirmation::confirm_tool, draft::change_draft, model::run_turn, reject};
use crate::host::{CoreHost, TurnKey};
use crate::store::{FinishTurn, TurnStart, UpdateSessionConfig};

pub(crate) async fn dispatch_action(host: &CoreHost, peer_id: u64, params: Value) {
    let Some(channel) = params
        .get("channel")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    let Some(client_seq) = params.get("clientSeq").and_then(Value::as_i64) else {
        return;
    };
    let Some(action_value) = params.get("action").cloned() else {
        return;
    };
    let action = serde_json::from_value::<StateAction>(action_value.clone())
        .unwrap_or(StateAction::Unknown(action_value.clone()));
    let origin = host.peer_origin(peer_id, client_seq).await;
    match action_value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "chat/turnStarted" => start_turn(host, &channel, action_value, action, origin).await,
        "chat/turnCancelled" => cancel_turn(host, &channel, action_value, action, origin).await,
        "chat/isReadChanged"
        | "chat/pendingMessageSet"
        | "chat/pendingMessageRemoved"
        | "chat/queuedMessagesReordered"
        | "chat/truncated" => super::pending::change_chat(host, &channel, action, origin).await,
        "chat/draftChanged" => change_draft(host, &channel, action_value, action, origin).await,
        "session/titleChanged"
        | "session/isReadChanged"
        | "session/isArchivedChanged"
        | "session/activeClientSet"
        | "session/activeClientRemoved" => {
            super::session::change_session(host, peer_id, &channel, action, origin).await
        }
        "session/configChanged" => {
            change_session_config(host, peer_id, &channel, action_value, action, origin).await
        }
        "chat/toolCallComplete" | "chat/toolCallContentChanged" => {
            super::client::change_client_tool(host, peer_id, &channel, action, origin).await
        }
        "chat/toolCallConfirmed" => {
            confirm_tool(host, peer_id, &channel, action_value, action, origin).await
        }
        _ => {
            reject(
                host,
                channel,
                action,
                origin,
                "action is outside this host's supported lifecycle",
            )
            .await
        }
    }
}

async fn start_turn(
    host: &CoreHost,
    channel: &str,
    raw: Value,
    action: StateAction,
    origin: Option<ahp_types::actions::ActionOrigin>,
) {
    let start = match serde_json::from_value::<ChatTurnStartedAction>(raw) {
        Ok(start)
            if !start.turn_id.trim().is_empty()
                && matches!(start.message.origin.kind, MessageKind::User) =>
        {
            start
        }
        Ok(_) => {
            reject(
                host,
                channel.into(),
                action,
                origin,
                "turnId must be non-empty and only a user message can start a turn",
            )
            .await;
            return;
        }
        Err(error) => {
            reject(
                host,
                channel.into(),
                action,
                origin,
                format!("invalid chat/turnStarted: {error}"),
            )
            .await;
            return;
        }
    };
    let _guard = host.inner.publish_lock.lock().await;
    if let Err(error) = start_turn_locked(host, channel, start, origin.clone()).await {
        drop(_guard);
        reject(host, channel.into(), action, origin, error).await;
    }
}

pub(crate) async fn start_turn_locked(
    host: &CoreHost,
    channel: &str,
    mut start: ChatTurnStartedAction,
    origin: Option<ahp_types::actions::ActionOrigin>,
) -> Result<(), String> {
    if host.inner.shutdown.token().is_cancelled() {
        return Err("host is shutting down".into());
    }
    if start.turn_id.trim().is_empty()
        || !matches!(start.message.origin.kind, MessageKind::User)
        || chrono::DateTime::parse_from_rfc3339(&start.started_at).is_err()
    {
        return Err("turn requires a non-empty id, valid startedAt, and a user message".into());
    }
    let selection = match start.message.model.clone() {
        Some(selection) => selection,
        None => host
            .inner
            .store
            .model_selection_for_chat(channel, host.inner.model_catalog.default_selection.as_ref())
            .map_err(|error| error.to_string())?,
    };
    host.inner
        .model_catalog
        .validate_selection(&selection)
        .map_err(|error| error.to_string())?;
    start.message.model = Some(selection);
    let transition = host
        .inner
        .store
        .start_turn(TurnStart {
            chat: channel.into(),
            action: start,
            origin,
            legacy_default_selection: host.inner.model_catalog.default_selection.clone(),
        })
        .map_err(|error| error.to_string())?;
    host.fanout(&transition).await;
    let started = transition.value;
    let cancellation = CancellationToken::new();
    host.inner.cancellations.lock().await.insert(
        TurnKey {
            chat: started.chat.clone(),
            turn_id: started.turn_id.clone(),
        },
        cancellation.clone(),
    );
    host.inner
        .turn_tasks
        .lock()
        .await
        .spawn(run_turn(host.clone(), started, cancellation));
    Ok(())
}

async fn change_session_config(
    host: &CoreHost,
    peer_id: u64,
    channel: &str,
    raw: Value,
    action: StateAction,
    origin: Option<ahp_types::actions::ActionOrigin>,
) {
    if !channel.starts_with("ahp-session:/") || !host.peer_is_subscribed(peer_id, channel).await {
        reject(
            host,
            channel.into(),
            action,
            origin,
            "session/configChanged requires a subscribed session channel",
        )
        .await;
        return;
    }
    let changed = match serde_json::from_value::<SessionConfigChangedAction>(raw) {
        Ok(changed)
            if changed
                .config
                .get("model")
                .and_then(Value::as_object)
                .and_then(|model| model.get("id"))
                .and_then(Value::as_str)
                .is_some() =>
        {
            changed
        }
        Ok(_) => {
            reject(
                host,
                channel.into(),
                action,
                origin,
                "session/configChanged requires the complete model selection",
            )
            .await;
            return;
        }
        Err(error) => {
            reject(
                host,
                channel.into(),
                action,
                origin,
                format!("invalid session/configChanged: {error}"),
            )
            .await;
            return;
        }
    };
    let selection = match host
        .inner
        .model_catalog
        .require_selection(Some(&changed.config))
    {
        Ok(selection) => selection,
        Err(error) => {
            reject(host, channel.into(), action, origin, error).await;
            return;
        }
    };
    let _guard = host.inner.publish_lock.lock().await;
    match host.inner.store.update_session_config(UpdateSessionConfig {
        session: channel.into(),
        model_selection: selection,
        origin: origin.clone(),
    }) {
        Ok(transition) => host.fanout(&transition).await,
        Err(error) => {
            drop(_guard);
            reject(host, channel.into(), action, origin, error).await;
        }
    }
}

async fn cancel_turn(
    host: &CoreHost,
    channel: &str,
    raw: Value,
    action: StateAction,
    origin: Option<ahp_types::actions::ActionOrigin>,
) {
    let Some(turn_id) = raw
        .get("turnId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
    else {
        reject(
            host,
            channel.into(),
            action,
            origin,
            "chat/turnCancelled requires a non-empty turnId",
        )
        .await;
        return;
    };
    let _guard = host.inner.publish_lock.lock().await;
    let key = TurnKey {
        chat: channel.into(),
        turn_id: turn_id.clone(),
    };
    if let Some(token) = host.inner.cancellations.lock().await.remove(&key) {
        token.cancel();
    }
    match host.inner.store.finish_turn(FinishTurn::Cancelled {
        chat: channel.into(),
        turn_id,
        origin: origin.clone(),
    }) {
        Ok(transition) => {
            host.fanout(&transition).await;
            let _ = super::pending::start_next_queued_locked(host, channel).await;
        }
        Err(error) => {
            drop(_guard);
            reject(host, channel.into(), action, origin, error).await;
        }
    }
}
