//! Turn orchestration split into routing, model, tool, and approval services.

mod confirmation;
mod dispatch;
mod draft;
mod history;
mod model;
mod tool;

use ahp_types::actions::StateAction;
use ahp_types::state::{ConfirmationOption, ToolCallConfirmationReason};
use openai_agent_model::ModelError;
use tokio_util::sync::CancellationToken;
use workspace_agent_tools::ToolExecution;

use crate::host::CoreHost;

const MAX_TOOL_ARGUMENT_BYTES: usize = 64 * 1024;
const MAX_TOOL_CALLS_PER_TURN: usize = 8;
const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

pub(crate) use dispatch::dispatch_action;

/// Decision delivered to an in-memory tool approval waiter.
pub(crate) enum ApprovalDecision {
    Approved {
        confirmed: ToolCallConfirmationReason,
        selected: Option<ConfirmationOption>,
    },
    Denied(ToolExecution),
}

pub(crate) enum TurnOutcome {
    Complete {
        model_cursor: Option<crate::ModelCursor>,
    },
    Cancelled,
    Failed(TurnFailure),
}

pub(crate) struct TurnFailure {
    pub(crate) error_type: &'static str,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl TurnFailure {
    pub(crate) fn model(error: ModelError) -> Self {
        let code = if matches!(error, ModelError::InvalidResponse(_)) {
            "AHP-MODEL-002"
        } else {
            "AHP-MODEL-001"
        };
        Self {
            error_type: "openai_upstream_error",
            code,
            message: error.to_string(),
        }
    }

    pub(crate) fn tool(message: impl ToString) -> Self {
        Self {
            error_type: "server_tool_error",
            code: "AHP-TOOL-001",
            message: message.to_string(),
        }
    }
}

pub(crate) async fn reject(
    host: &CoreHost,
    channel: String,
    action: StateAction,
    origin: Option<ahp_types::actions::ActionOrigin>,
    reason: impl ToString,
) {
    let _guard = host.inner.publish_lock.lock().await;
    if let Ok(transition) =
        host.inner
            .store
            .reject_action(channel, action, origin, reason.to_string())
    {
        host.fanout(&transition).await;
    }
}

pub(crate) fn cancelled(host: &CoreHost, cancellation: &CancellationToken) -> bool {
    cancellation.is_cancelled() || host.inner.shutdown.token().is_cancelled()
}

pub(crate) fn split_scalars(text: &str, limit: usize) -> Vec<String> {
    text.chars()
        .collect::<Vec<_>>()
        .chunks(limit)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::split_scalars;

    #[test]
    fn scalar_chunks_keep_unicode_scalars_together() {
        assert_eq!(split_scalars("é🙂z", 2), vec!["é🙂", "z"]);
    }
}
