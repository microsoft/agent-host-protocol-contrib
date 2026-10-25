//! Pending-input acceptance, delivery, and automatic turn startup.

use ahp_types::actions::{ChatPendingMessageRemovedAction, ChatTurnStartedAction, StateAction};
use ahp_types::state::{PendingMessageKind, SnapshotState};
use openai_agent_model::{ModelMessage, ModelRequest};
use uuid::Uuid;

use super::{dispatch::start_turn_locked, reject};
use crate::host::CoreHost;
use crate::{ChangeChat, ChatMutation, SnapshotRequest};

pub(super) async fn change_chat(
    host: &CoreHost,
    channel: &str,
    action: StateAction,
    origin: Option<ahp_types::actions::ActionOrigin>,
) {
    let mutation = match &action {
        StateAction::ChatIsReadChanged(value) => ChatMutation::IsRead(value.is_read),
        StateAction::ChatPendingMessageSet(value) => ChatMutation::PendingSet(value.clone()),
        StateAction::ChatPendingMessageRemoved(value) => {
            ChatMutation::PendingRemoved(value.clone())
        }
        StateAction::ChatQueuedMessagesReordered(value) => ChatMutation::Reordered(value.clone()),
        StateAction::ChatTruncated(value) => ChatMutation::Truncated(value.clone()),
        _ => {
            reject(
                host,
                channel.into(),
                action,
                origin,
                "invalid chat state action",
            )
            .await;
            return;
        }
    };
    if let ChatMutation::PendingSet(value) = &mutation {
        if let Some(selection) = &value.message.model {
            if let Err(error) = host.inner.model_catalog.validate_selection(selection) {
                reject(host, channel.into(), action, origin, error).await;
                return;
            }
        }
    }
    let _guard = host.inner.publish_lock.lock().await;
    match host.inner.store.change_chat(ChangeChat {
        chat: channel.into(),
        mutation: mutation.clone(),
        origin: origin.clone(),
    }) {
        Ok(transition) => {
            if matches!(mutation, ChatMutation::Truncated(_)) {
                host.inner.cancellations.lock().await.retain(|key, token| {
                    if key.chat == channel {
                        token.cancel();
                        false
                    } else {
                        true
                    }
                });
                host.inner
                    .approvals
                    .lock()
                    .await
                    .retain(|key, _| key.chat != channel);
            }
            host.fanout(&transition).await;
            if !matches!(mutation, ChatMutation::IsRead(_)) {
                let _ = start_next_queued_locked(host, channel).await;
            }
        }
        Err(error) => {
            drop(_guard);
            reject(host, channel.into(), action, origin, error).await;
        }
    }
}

pub(super) async fn start_next_queued_locked(host: &CoreHost, channel: &str) -> Result<(), String> {
    let snapshot = host
        .inner
        .store
        .snapshot(SnapshotRequest {
            channel: channel.into(),
            model_catalog: host.inner.model_catalog.clone(),
            server_tools: host.inner.profile.server_tools(),
        })
        .map_err(|error| error.to_string())?;
    let SnapshotState::Chat(chat) = snapshot.state else {
        return Ok(());
    };
    if chat.active_turn.is_some() {
        return Ok(());
    }
    let pending = host
        .inner
        .store
        .pending_messages(channel)
        .map_err(|error| error.to_string())?;
    let Some(entry) = pending.entries.into_iter().next() else {
        return Ok(());
    };
    start_turn_locked(
        host,
        channel,
        ChatTurnStartedAction {
            turn_id: Uuid::new_v4().to_string(),
            started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            message: entry.message,
            queued_message_id: Some(entry.id),
            meta: None,
        },
        None,
    )
    .await
}

pub(super) async fn deliver_steering(
    host: &CoreHost,
    started: &crate::StartedTurn,
    cancellation: &tokio_util::sync::CancellationToken,
    request: &mut ModelRequest,
) -> Result<(), String> {
    if super::cancelled(host, cancellation) {
        return Ok(());
    }
    let _guard = host.inner.publish_lock.lock().await;
    if super::cancelled(host, cancellation) {
        return Ok(());
    }
    let pending = host
        .inner
        .store
        .pending_messages(&started.chat)
        .map_err(|error| error.to_string())?;
    if let Some(entry) = pending.steering {
        if let Some(context) =
            super::history::attachment_message(&entry.message, Some(&pending.steering_attachments))
        {
            request.messages.push(context);
        }
        request.messages.push(ModelMessage::User {
            content: entry.message.text,
        });
        let transition = host
            .inner
            .store
            .change_chat(ChangeChat {
                chat: started.chat.clone(),
                mutation: ChatMutation::PendingRemoved(ChatPendingMessageRemovedAction {
                    kind: PendingMessageKind::Steering,
                    id: entry.id,
                }),
                origin: None,
            })
            .map_err(|error| error.to_string())?;
        host.fanout(&transition).await;
    }
    Ok(())
}
