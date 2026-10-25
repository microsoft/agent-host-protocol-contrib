//! Client-executed tools, ownership validation, and cancellation-aware waiters.

use ahp_types::actions::{ActionOrigin, StateAction};
use ahp_types::state::{SnapshotState, ToolCallConfirmationReason, ToolCallResult};
use openai_agent_model::ModelToolCall;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::{cancelled, reject, TurnFailure, TurnOutcome};
use crate::host::CoreHost;
use crate::presentation::ToolPresentation;
use crate::{
    ApprovalKey, ChangeClientTool, ClientTool, ClientToolMutation, SnapshotRequest,
    ToolCallTransition,
};

pub(crate) struct PendingClientTool {
    pub(crate) session: String,
    pub(crate) client_id: String,
    sender: oneshot::Sender<ToolCallResult>,
}

pub(super) async fn execute_client_tool(
    host: &CoreHost,
    started: &crate::StartedTurn,
    tool: &ClientTool,
    call: ModelToolCall,
    cancellation: &CancellationToken,
) -> Result<(String, String), TurnOutcome> {
    let key = ApprovalKey {
        chat: started.chat.clone(),
        turn_id: started.turn_id.clone(),
        tool_call_id: call.id.clone(),
    };
    let presentation =
        ToolPresentation::for_turn(call.id.clone(), call.name, call.arguments, started)
            .map_err(TurnFailure::tool)
            .map_err(TurnOutcome::Failed)?;
    let (sender, mut receiver) = oneshot::channel();
    {
        let _guard = host.inner.publish_lock.lock().await;
        if cancelled(host, cancellation) {
            return Err(TurnOutcome::Cancelled);
        }
        let snapshot = host
            .inner
            .store
            .snapshot(SnapshotRequest {
                channel: started.session.clone(),
                model_catalog: host.inner.model_catalog.clone(),
                server_tools: Vec::new(),
            })
            .map_err(TurnFailure::tool)
            .map_err(TurnOutcome::Failed)?;
        let registered = matches!(snapshot.state, SnapshotState::Session(state) if state.active_clients.iter().any(|client|
            client.client_id == tool.client_id && client.tools.iter().any(|definition| definition.name == tool.definition.name)));
        let connected = host.inner.peers.lock().await.values().any(|peer| {
            peer.client_id.as_deref() == Some(tool.client_id.as_str())
                && peer.subscriptions.contains(&started.session)
        });
        let transition = host
            .inner
            .store
            .transition_tool(ToolCallTransition {
                chat: started.chat.clone(),
                turn_id: started.turn_id.clone(),
                tool_call_id: call.id.clone(),
                next_state: presentation.ready_state(ToolCallConfirmationReason::NotNeeded, None),
                actions: vec![presentation.ready_action(
                    &started.turn_id,
                    Some(ToolCallConfirmationReason::NotNeeded),
                    None,
                )],
                origin: None,
                allow_command: None,
            })
            .map_err(TurnFailure::tool)
            .map_err(TurnOutcome::Failed)?;
        host.inner.client_tools.lock().await.insert(
            key.clone(),
            PendingClientTool {
                session: started.session.clone(),
                client_id: tool.client_id.clone(),
                sender,
            },
        );
        host.fanout(&transition).await;
        if !connected {
            fail_client_tools_locked(host, &started.session, &tool.client_id).await;
        } else if !registered {
            fail_client_tool_locked(
                host,
                &key,
                "Client tool is no longer registered by its owner.",
            )
            .await;
        }
    }
    let result = tokio::select! {
        biased;
        _ = cancellation.cancelled() => None,
        _ = host.inner.shutdown.token().cancelled() => None,
        result = &mut receiver => Some(result.unwrap_or_else(|_| failed_result("Client tool executor disconnected."))),
        _ = tokio::time::sleep(super::APPROVAL_TIMEOUT) => {
            let _guard = host.inner.publish_lock.lock().await;
            if cancelled(host, cancellation) { None } else {
                let result = failed_result("Client tool execution timed out.");
                if host.inner.client_tools.lock().await.remove(&key).is_some() {
                    let transition = host.inner.store.change_client_tool(ChangeClientTool {
                        chat: key.chat.clone(), turn_id: key.turn_id.clone(), tool_call_id: key.tool_call_id.clone(),
                        client_id: tool.client_id.clone(), mutation: ClientToolMutation::Complete(result.clone()), origin: None,
                    }).map_err(TurnFailure::tool).map_err(TurnOutcome::Failed)?;
                    host.fanout(&transition).await;
                    Some(result)
                } else {
                    Some(receiver.try_recv().unwrap_or(result))
                }
            }
        }
    };
    host.inner.client_tools.lock().await.remove(&key);
    match result {
        Some(result) => Ok((
            call.id,
            serde_json::to_string(&result)
                .map_err(TurnFailure::tool)
                .map_err(TurnOutcome::Failed)?,
        )),
        None => Err(TurnOutcome::Cancelled),
    }
}

pub(super) async fn change_client_tool(
    host: &CoreHost,
    peer_id: u64,
    channel: &str,
    action: StateAction,
    origin: Option<ActionOrigin>,
) {
    let (turn_id, tool_call_id, mutation) = match &action {
        StateAction::ChatToolCallComplete(complete)
            if complete.requires_result_confirmation != Some(true) =>
        {
            (
                complete.turn_id.clone(),
                complete.tool_call_id.clone(),
                ClientToolMutation::Complete(complete.result.clone()),
            )
        }
        StateAction::ChatToolCallContentChanged(changed) => (
            changed.turn_id.clone(),
            changed.tool_call_id.clone(),
            ClientToolMutation::Content(changed.content.clone()),
        ),
        _ => {
            reject(
                host,
                channel.into(),
                action,
                origin,
                "unsupported client tool update",
            )
            .await;
            return;
        }
    };
    let key = ApprovalKey {
        chat: channel.into(),
        turn_id: turn_id.clone(),
        tool_call_id: tool_call_id.clone(),
    };
    let _guard = host.inner.publish_lock.lock().await;
    let pending = host
        .inner
        .client_tools
        .lock()
        .await
        .get(&key)
        .map(|pending| (pending.client_id.clone(), pending.session.clone()));
    let Some((client_id, session)) = pending.filter(|(client_id, _)| {
        origin
            .as_ref()
            .is_some_and(|origin| origin.client_id == *client_id)
    }) else {
        drop(_guard);
        reject(
            host,
            channel.into(),
            action,
            origin,
            "tool updates must come from the running tool's owning client",
        )
        .await;
        return;
    };
    if !host.peer_is_subscribed(peer_id, channel).await
        && !host.peer_is_subscribed(peer_id, &session).await
    {
        drop(_guard);
        reject(
            host,
            channel.into(),
            action,
            origin,
            "client tool updates require a subscribed chat or owning session",
        )
        .await;
        return;
    }
    match host.inner.store.change_client_tool(ChangeClientTool {
        chat: channel.into(),
        turn_id,
        tool_call_id,
        client_id,
        mutation: mutation.clone(),
        origin: origin.clone(),
    }) {
        Ok(transition) => {
            host.fanout(&transition).await;
            if let ClientToolMutation::Complete(result) = mutation {
                if let Some(pending) = host.inner.client_tools.lock().await.remove(&key) {
                    let _ = pending.sender.send(result);
                }
            }
        }
        Err(error) => {
            drop(_guard);
            reject(host, channel.into(), action, origin, error).await;
        }
    }
}

pub(crate) async fn fail_client_tools_locked(host: &CoreHost, session: &str, client_id: &str) {
    let keys = host
        .inner
        .client_tools
        .lock()
        .await
        .iter()
        .filter(|(_, pending)| pending.session == session && pending.client_id == client_id)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    for key in keys {
        fail_client_tool_locked(
            host,
            &key,
            "Client tool owner left the session or disconnected.",
        )
        .await;
    }
}

async fn fail_client_tool_locked(host: &CoreHost, key: &ApprovalKey, message: &str) {
    let client_id = host
        .inner
        .client_tools
        .lock()
        .await
        .get(key)
        .map(|pending| pending.client_id.clone());
    let Some(client_id) = client_id else { return };
    let result = failed_result(message);
    if let Ok(transition) = host.inner.store.change_client_tool(ChangeClientTool {
        chat: key.chat.clone(),
        turn_id: key.turn_id.clone(),
        tool_call_id: key.tool_call_id.clone(),
        client_id,
        mutation: ClientToolMutation::Complete(result.clone()),
        origin: None,
    }) {
        host.fanout(&transition).await;
    }
    if let Some(pending) = host.inner.client_tools.lock().await.remove(key) {
        let _ = pending.sender.send(result);
    }
}

fn failed_result(message: &str) -> ToolCallResult {
    workspace_agent_tools::ToolExecution::failed(message).to_call_result()
}

#[cfg(test)]
mod tests;
