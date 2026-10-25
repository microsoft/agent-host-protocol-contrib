//! Atomic tool lifecycle transitions and sequential execution.

use ahp_types::state::{ConfirmationOption, ResponsePart, ToolCallConfirmationReason};
use openai_agent_model::ModelToolCall;
use tokio_util::sync::CancellationToken;
use workspace_agent_tools::{ExecutionOutcome, ToolExecution, ToolInvocation};

use super::{cancelled, confirmation::await_approval, ApprovalDecision, TurnFailure, TurnOutcome};
use crate::host::CoreHost;
use crate::presentation::ToolPresentation;
use crate::store::ToolCallTransition;

pub(crate) async fn append_text(
    host: &CoreHost,
    started: &crate::StartedTurn,
    part_id: String,
    chunk: String,
    cancellation: &CancellationToken,
) -> Result<(), TurnOutcome> {
    let _guard = host.inner.publish_lock.lock().await;
    if cancelled(host, cancellation) {
        return Err(TurnOutcome::Cancelled);
    }
    let transition = host
        .inner
        .store
        .append_text(&started.chat, &started.turn_id, &part_id, &chunk)
        .map_err(TurnFailure::tool)
        .map_err(TurnOutcome::Failed)?;
    host.fanout(&transition).await;
    Ok(())
}

pub(crate) async fn append_response_part(
    host: &CoreHost,
    started: &crate::StartedTurn,
    part: ResponsePart,
    cancellation: &CancellationToken,
) -> Result<(), TurnOutcome> {
    let _guard = host.inner.publish_lock.lock().await;
    if cancelled(host, cancellation) {
        return Err(TurnOutcome::Cancelled);
    }
    let transition = host
        .inner
        .store
        .append_response_part(&started.chat, &started.turn_id, part)
        .map_err(TurnFailure::tool)
        .map_err(TurnOutcome::Failed)?;
    host.fanout(&transition).await;
    Ok(())
}

pub(crate) async fn start_tool(
    host: &CoreHost,
    started: &crate::StartedTurn,
    presentation: &ToolPresentation,
    cancellation: &CancellationToken,
) -> Result<(), TurnOutcome> {
    let _guard = host.inner.publish_lock.lock().await;
    if cancelled(host, cancellation) {
        return Err(TurnOutcome::Cancelled);
    }
    let transition = host
        .inner
        .store
        .start_tool(
            &started.chat,
            &started.turn_id,
            presentation.response_part(),
            presentation.start_action(&started.turn_id),
        )
        .map_err(TurnFailure::tool)
        .map_err(TurnOutcome::Failed)?;
    host.fanout(&transition).await;
    Ok(())
}

pub(crate) async fn append_tool_delta(
    host: &CoreHost,
    started: &crate::StartedTurn,
    presentation: &ToolPresentation,
    delta: String,
    cancellation: &CancellationToken,
) -> Result<(), TurnOutcome> {
    let _guard = host.inner.publish_lock.lock().await;
    if cancelled(host, cancellation) {
        return Err(TurnOutcome::Cancelled);
    }
    let action = presentation.delta_action(&started.turn_id, delta.clone());
    let transition = host
        .inner
        .store
        .append_tool_delta(
            &started.chat,
            &started.turn_id,
            &presentation.id,
            &delta,
            action,
        )
        .map_err(TurnFailure::tool)
        .map_err(TurnOutcome::Failed)?;
    host.fanout(&transition).await;
    Ok(())
}

pub(crate) async fn execute_tool(
    host: &CoreHost,
    started: &crate::StartedTurn,
    workspace: &workspace_agent_tools::SelectedWorkspace,
    call: ModelToolCall,
    cancellation: &CancellationToken,
) -> Result<(String, String), TurnOutcome> {
    if let Some(tool) = started
        .client_tools
        .iter()
        .find(|tool| tool.definition.name == call.name)
    {
        return super::client::execute_client_tool(host, started, tool, call, cancellation).await;
    }
    let presentation =
        ToolPresentation::new(call.id.clone(), call.name.clone(), call.arguments.clone())
            .map_err(TurnFailure::tool)
            .map_err(TurnOutcome::Failed)?;
    let prepared = match host
        .inner
        .profile
        .preflight(workspace, ToolInvocation::new(call.name, call.arguments))
    {
        Ok(prepared) => prepared,
        Err(error) => {
            let result = ToolExecution::failed(error.to_string());
            transition_ready(
                host,
                started,
                &presentation,
                ToolCallConfirmationReason::NotNeeded,
                None,
                cancellation,
            )
            .await?;
            complete_tool(
                host,
                started,
                &presentation,
                &result,
                ToolCallConfirmationReason::NotNeeded,
                None,
                cancellation,
            )
            .await?;
            return Ok((call.id, result.model_content()));
        }
    };
    let (confirmed, selected) = match host.inner.profile.exact_command_approval(&prepared) {
        Some(command)
            if host
                .inner
                .store
                .is_command_allowed(&command)
                .map_err(TurnFailure::tool)
                .map_err(TurnOutcome::Failed)? =>
        {
            transition_ready(
                host,
                started,
                &presentation,
                ToolCallConfirmationReason::Setting,
                None,
                cancellation,
            )
            .await?;
            (ToolCallConfirmationReason::Setting, None)
        }
        Some(command) => {
            match await_approval(host, started, presentation.clone(), command, cancellation).await?
            {
                ApprovalDecision::Approved {
                    confirmed,
                    selected,
                } => (confirmed, selected),
                ApprovalDecision::Denied(result) => return Ok((call.id, result.model_content())),
            }
        }
        None => {
            transition_ready(
                host,
                started,
                &presentation,
                ToolCallConfirmationReason::NotNeeded,
                None,
                cancellation,
            )
            .await?;
            (ToolCallConfirmationReason::NotNeeded, None)
        }
    };
    let result = host
        .inner
        .profile
        .execute(
            workspace,
            prepared,
            cancellation,
            host.inner.shutdown.token(),
        )
        .await;
    let ExecutionOutcome::Completed(result) = result else {
        return Err(TurnOutcome::Cancelled);
    };
    complete_tool(
        host,
        started,
        &presentation,
        &result,
        confirmed,
        selected,
        cancellation,
    )
    .await?;
    Ok((call.id, result.model_content()))
}

async fn transition_ready(
    host: &CoreHost,
    started: &crate::StartedTurn,
    presentation: &ToolPresentation,
    confirmed: ToolCallConfirmationReason,
    selected: Option<ConfirmationOption>,
    cancellation: &CancellationToken,
) -> Result<(), TurnOutcome> {
    transition_tool(
        host,
        ToolCallTransition {
            chat: started.chat.clone(),
            turn_id: started.turn_id.clone(),
            tool_call_id: presentation.id.clone(),
            next_state: presentation.ready_state(confirmed.clone(), selected.clone()),
            actions: vec![presentation.ready_action(&started.turn_id, Some(confirmed), None)],
            origin: None,
            allow_command: None,
        },
        cancellation,
    )
    .await
}

async fn complete_tool(
    host: &CoreHost,
    started: &crate::StartedTurn,
    presentation: &ToolPresentation,
    result: &ToolExecution,
    confirmed: ToolCallConfirmationReason,
    selected: Option<ConfirmationOption>,
    cancellation: &CancellationToken,
) -> Result<(), TurnOutcome> {
    transition_tool(
        host,
        ToolCallTransition {
            chat: started.chat.clone(),
            turn_id: started.turn_id.clone(),
            tool_call_id: presentation.id.clone(),
            next_state: presentation.completed_state(result, confirmed, selected),
            actions: vec![presentation.complete_action(&started.turn_id, result)],
            origin: None,
            allow_command: None,
        },
        cancellation,
    )
    .await
}

async fn transition_tool(
    host: &CoreHost,
    transition: ToolCallTransition,
    cancellation: &CancellationToken,
) -> Result<(), TurnOutcome> {
    let _guard = host.inner.publish_lock.lock().await;
    if cancelled(host, cancellation) {
        return Err(TurnOutcome::Cancelled);
    }
    let committed = host
        .inner
        .store
        .transition_tool(transition)
        .map_err(TurnFailure::tool)
        .map_err(TurnOutcome::Failed)?;
    host.fanout(&committed).await;
    Ok(())
}
