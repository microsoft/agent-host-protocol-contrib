//! Validation and dispatch of client lifecycle actions.

use ahp_types::actions::{ChatTurnStartedAction, StateAction};
use ahp_types::state::MessageKind;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::{confirmation::confirm_tool, model::run_turn, reject};
use crate::host::{CoreHost, TurnKey};
use crate::store::{FinishTurn, TurnStart};

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
    if host.inner.shutdown.token().is_cancelled() {
        return;
    }
    match host.inner.store.start_turn(TurnStart {
        chat: channel.into(),
        action: start,
        origin: origin.clone(),
    }) {
        Ok(transition) => {
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
            let task_host = host.clone();
            host.inner
                .turn_tasks
                .lock()
                .await
                .spawn(async move { run_turn(task_host, started, cancellation).await });
        }
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
        Ok(transition) => host.fanout(&transition).await,
        Err(error) => {
            drop(_guard);
            reject(host, channel.into(), action, origin, error).await;
        }
    }
}
