//! Confirmation waiter registration and exact-command decisions.

use ahp_types::actions::{ChatToolCallConfirmedAction, StateAction};
use ahp_types::state::{
    ConfirmationOption, ConfirmationOptionKind, ToolCallCancellationReason,
    ToolCallConfirmationReason,
};
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use workspace_agent_tools::{ExactCommandApproval, ToolExecution};

use super::{cancelled, reject, ApprovalDecision, TurnFailure, TurnOutcome, APPROVAL_TIMEOUT};
use crate::host::{CoreHost, PendingApproval};
use crate::presentation::ToolPresentation;
use crate::store::{ApprovalKey, ToolCallTransition};

pub(crate) async fn await_approval(
    host: &CoreHost,
    started: &crate::StartedTurn,
    presentation: ToolPresentation,
    command: ExactCommandApproval,
    cancellation: &CancellationToken,
) -> Result<ApprovalDecision, TurnOutcome> {
    await_approval_for(
        host,
        started,
        presentation,
        command,
        cancellation,
        APPROVAL_TIMEOUT,
    )
    .await
}

async fn await_approval_for(
    host: &CoreHost,
    started: &crate::StartedTurn,
    presentation: ToolPresentation,
    command: ExactCommandApproval,
    cancellation: &CancellationToken,
    timeout: std::time::Duration,
) -> Result<ApprovalDecision, TurnOutcome> {
    let key = ApprovalKey {
        chat: started.chat.clone(),
        turn_id: started.turn_id.clone(),
        tool_call_id: presentation.id.clone(),
    };
    let waiter_id = Uuid::new_v4();
    let (sender, receiver) = oneshot::channel();
    {
        let _guard = host.inner.publish_lock.lock().await;
        if cancelled(host, cancellation) {
            return Err(TurnOutcome::Cancelled);
        }
        let options = confirmation_options();
        let transition = ToolCallTransition {
            chat: started.chat.clone(),
            turn_id: started.turn_id.clone(),
            tool_call_id: presentation.id.clone(),
            next_state: presentation.pending_state(options.clone()),
            actions: vec![presentation.ready_action(&started.turn_id, None, Some(options))],
            origin: None,
            allow_command: None,
        };
        let committed = host
            .inner
            .store
            .transition_tool(transition)
            .map_err(TurnFailure::tool)
            .map_err(TurnOutcome::Failed)?;
        host.inner.approvals.lock().await.insert(
            key.clone(),
            PendingApproval {
                waiter_id,
                presentation,
                command,
                sender,
            },
        );
        host.fanout(&committed).await;
    }
    let timer = tokio::time::sleep(timeout);
    tokio::pin!(timer);
    let decision = tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(TurnOutcome::Cancelled),
        _ = host.inner.shutdown.token().cancelled() => Err(TurnOutcome::Cancelled),
        _ = &mut timer => Err(TurnOutcome::Failed(TurnFailure::tool("shell confirmation timed out after five minutes"))),
        received = receiver => received.map_err(|_| TurnOutcome::Failed(TurnFailure::tool("shell confirmation waiter was cancelled"))),
    };
    remove_waiter(host, &key, waiter_id).await;
    decision
}

async fn remove_waiter(host: &CoreHost, key: &ApprovalKey, waiter_id: Uuid) {
    let mut waiters = host.inner.approvals.lock().await;
    if waiters
        .get(key)
        .is_some_and(|waiter| waiter.waiter_id == waiter_id)
    {
        waiters.remove(key);
    }
}

pub(crate) async fn confirm_tool(
    host: &CoreHost,
    peer_id: u64,
    channel: &str,
    raw: Value,
    action: StateAction,
    origin: Option<ahp_types::actions::ActionOrigin>,
) {
    if !host.peer_is_subscribed(peer_id, channel).await {
        reject(
            host,
            channel.into(),
            action,
            origin,
            "tool confirmations require an initialized client subscribed to this chat",
        )
        .await;
        return;
    }
    let confirmation = match serde_json::from_value::<ChatToolCallConfirmedAction>(raw) {
        Ok(value) => value,
        Err(error) => {
            reject(
                host,
                channel.into(),
                action,
                origin,
                format!("invalid chat/toolCallConfirmed: {error}"),
            )
            .await;
            return;
        }
    };
    let Some(option_id) = confirmation.selected_option_id.as_deref() else {
        reject(
            host,
            channel.into(),
            action,
            origin,
            "tool confirmation must select an advertised option",
        )
        .await;
        return;
    };
    if confirmation.turn_id.trim().is_empty()
        || confirmation.tool_call_id.trim().is_empty()
        || confirmation.edited_tool_input.is_some()
        || !valid_confirmation(&confirmation, option_id)
    {
        reject(
            host,
            channel.into(),
            action,
            origin,
            "invalid shell tool confirmation",
        )
        .await;
        return;
    }
    let key = ApprovalKey {
        chat: channel.into(),
        turn_id: confirmation.turn_id.clone(),
        tool_call_id: confirmation.tool_call_id.clone(),
    };
    let _guard = host.inner.publish_lock.lock().await;
    let pending = {
        let waiters = host.inner.approvals.lock().await;
        waiters
            .get(&key)
            .map(|waiter| (waiter.presentation.clone(), waiter.command.clone()))
    };
    let Some((presentation, command)) = pending else {
        drop(_guard);
        reject(
            host,
            channel.into(),
            action,
            origin,
            "no matching shell confirmation is pending",
        )
        .await;
        return;
    };
    let option = confirmation_option(option_id);
    let (next_state, actions, decision, allow_command) = match option_id {
        "allow-once" | "always-allow-command" => {
            let reason = if option_id == "allow-once" {
                ToolCallConfirmationReason::UserAction
            } else {
                ToolCallConfirmationReason::Setting
            };
            let canonical =
                canonical_confirmation(&confirmation, true, Some(reason.clone()), None, option_id);
            (
                presentation.ready_state(reason.clone(), Some(option.clone())),
                vec![StateAction::ChatToolCallConfirmed(canonical)],
                ApprovalDecision::Approved {
                    confirmed: reason,
                    selected: Some(option),
                },
                (option_id == "always-allow-command").then_some(command),
            )
        }
        "deny" => {
            let result = ToolExecution::failed("Shell command was denied by the user.");
            let canonical = canonical_confirmation(
                &confirmation,
                false,
                None,
                Some(ToolCallCancellationReason::Denied),
                option_id,
            );
            (
                presentation.denied_state(option),
                vec![
                    StateAction::ChatToolCallConfirmed(canonical),
                    presentation.complete_action(&confirmation.turn_id, &result),
                ],
                ApprovalDecision::Denied(result),
                None,
            )
        }
        _ => {
            drop(_guard);
            reject(
                host,
                channel.into(),
                action,
                origin,
                "selectedOptionId is not an advertised shell confirmation option",
            )
            .await;
            return;
        }
    };
    let transition = ToolCallTransition {
        chat: channel.into(),
        turn_id: confirmation.turn_id,
        tool_call_id: confirmation.tool_call_id,
        next_state,
        actions,
        origin,
        allow_command,
    };
    match host.inner.store.transition_tool(transition) {
        Ok(committed) => {
            let waiter = host.inner.approvals.lock().await.remove(&key);
            host.fanout(&committed).await;
            if let Some(waiter) = waiter {
                let _ = waiter.sender.send(decision);
            }
        }
        Err(error) => {
            drop(_guard);
            reject(host, channel.into(), action, None, error).await;
        }
    }
}

fn confirmation_options() -> Vec<ConfirmationOption> {
    ["allow-once", "always-allow-command", "deny"]
        .into_iter()
        .map(confirmation_option)
        .collect()
}

fn confirmation_option(id: &str) -> ConfirmationOption {
    let (label, kind) = match id {
        "allow-once" => ("Allow once", ConfirmationOptionKind::Approve),
        "always-allow-command" => ("Always allow this command", ConfirmationOptionKind::Approve),
        _ => ("Deny", ConfirmationOptionKind::Deny),
    };
    ConfirmationOption {
        id: id.into(),
        label: label.into(),
        kind,
        group: None,
    }
}

fn valid_confirmation(value: &ChatToolCallConfirmedAction, option: &str) -> bool {
    match option {
        "allow-once" => {
            value.approved
                && value.reason.is_none()
                && value
                    .confirmed
                    .as_ref()
                    .is_none_or(|reason| *reason == ToolCallConfirmationReason::UserAction)
        }
        "always-allow-command" => {
            value.approved
                && value.reason.is_none()
                && value
                    .confirmed
                    .as_ref()
                    .is_none_or(|reason| *reason == ToolCallConfirmationReason::Setting)
        }
        "deny" => {
            !value.approved
                && value.confirmed.is_none()
                && value
                    .reason
                    .is_none_or(|reason| reason == ToolCallCancellationReason::Denied)
        }
        _ => false,
    }
}

fn canonical_confirmation(
    value: &ChatToolCallConfirmedAction,
    approved: bool,
    confirmed: Option<ToolCallConfirmationReason>,
    reason: Option<ToolCallCancellationReason>,
    option: &str,
) -> ChatToolCallConfirmedAction {
    ChatToolCallConfirmedAction {
        turn_id: value.turn_id.clone(),
        tool_call_id: value.tool_call_id.clone(),
        meta: None,
        approved,
        confirmed,
        reason,
        edited_tool_input: None,
        user_suggestion: None,
        reason_message: None,
        selected_option_id: Some(option.into()),
    }
}

#[cfg(test)]
mod tests;
