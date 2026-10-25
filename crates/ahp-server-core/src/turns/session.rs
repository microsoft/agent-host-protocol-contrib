//! Validation of session metadata and active-client actions.

use ahp_types::actions::{ActionOrigin, StateAction};

use super::reject;
use crate::host::CoreHost;
use crate::{ChangeSession, SessionMutation};

pub(super) async fn change_session(
    host: &CoreHost,
    peer_id: u64,
    channel: &str,
    action: StateAction,
    origin: Option<ActionOrigin>,
) {
    if !channel.starts_with("ahp-session:/") || !host.peer_is_subscribed(peer_id, channel).await {
        reject(
            host,
            channel.into(),
            action,
            origin,
            "session actions require a subscribed session channel",
        )
        .await;
        return;
    }
    let mutation = match &action {
        StateAction::SessionTitleChanged(changed) => SessionMutation::Title(changed.title.clone()),
        StateAction::SessionIsReadChanged(changed) => SessionMutation::IsRead(changed.is_read),
        StateAction::SessionIsArchivedChanged(changed) => {
            SessionMutation::IsArchived(changed.is_archived)
        }
        StateAction::SessionActiveClientSet(changed) => {
            SessionMutation::ActiveClientSet(changed.active_client.clone())
        }
        StateAction::SessionActiveClientRemoved(changed) => {
            SessionMutation::ActiveClientRemoved(changed.client_id.clone())
        }
        _ => {
            reject(
                host,
                channel.into(),
                action,
                origin,
                "invalid session action",
            )
            .await;
            return;
        }
    };
    let client_id = match &mutation {
        SessionMutation::ActiveClientSet(client) => Some(client.client_id.as_str()),
        SessionMutation::ActiveClientRemoved(client_id) => Some(client_id.as_str()),
        _ => None,
    };
    if client_id.is_some() && client_id != origin.as_ref().map(|origin| origin.client_id.as_str()) {
        reject(
            host,
            channel.into(),
            action,
            origin,
            "active client changes must identify the dispatching client",
        )
        .await;
        return;
    }
    let removed_client = match &mutation {
        SessionMutation::ActiveClientRemoved(client_id) => Some(client_id.clone()),
        _ => None,
    };
    let _guard = host.inner.publish_lock.lock().await;
    match host.inner.store.change_session(ChangeSession {
        session: channel.into(),
        mutation,
        origin: origin.clone(),
    }) {
        Ok(transition) => {
            host.fanout(&transition).await;
            if let Some(client_id) = removed_client {
                super::fail_client_tools_locked(host, channel, &client_id).await;
            }
        }
        Err(error) => {
            drop(_guard);
            reject(host, channel.into(), action, origin, error).await;
        }
    }
}
