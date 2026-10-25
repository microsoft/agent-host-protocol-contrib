//! Environment-facing workspace tool port and its standard adapter.

use std::future::Future;
use std::pin::Pin;

use ahp_types::common::Uri;
use ahp_types::state::ToolDefinition;
use openai_agent_model::ModelToolDefinition;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use workspace_agent_tools::{
    ConfiguredWorkspaces, ExactCommandApproval, ExecutionOutcome, PreparedTool, SelectedWorkspace,
    ToolError, ToolInvocation, WorkspaceError, WorkspaceTools,
};

/// Future returned by an [`AgentProfile`] tool execution.
pub type ProfileFuture<'a> = Pin<Box<dyn Future<Output = ExecutionOutcome> + Send + 'a>>;

/// Failure selecting the workspace associated with a session.
#[derive(Debug, Error)]
pub enum ProfileError {
    /// The selected workspace does not satisfy the configured profile.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

/// Host-owned behavior at the boundary to an agent environment.
///
/// Implementations own workspace selection and execute a preflighted operation
/// while honoring both supplied cancellation tokens.
pub trait AgentProfile: Send + Sync {
    /// Workspace roots available to resource requests; absent profiles deny access.
    fn resource_workspaces(&self) -> Option<&ConfiguredWorkspaces> {
        None
    }

    /// Selects one configured workspace for a new session.
    fn select_workspace(
        &self,
        requested: Option<Vec<Uri>>,
    ) -> Result<SelectedWorkspace, ProfileError>;

    /// AHP server tools exposed in session snapshots.
    fn server_tools(&self) -> Vec<ToolDefinition>;

    /// Model function definitions exposed for each completion round.
    fn model_tools(&self) -> Vec<ModelToolDefinition>;

    /// Validates an untrusted model tool call without executing it.
    fn preflight(
        &self,
        workspace: &SelectedWorkspace,
        invocation: ToolInvocation,
    ) -> Result<PreparedTool, ToolError>;

    /// Returns an approval identity only for an exact shell command.
    fn exact_command_approval(&self, prepared: &PreparedTool) -> Option<ExactCommandApproval>;

    /// Runs a prepared tool. Cancellation must stop the full child process tree.
    fn execute<'a>(
        &'a self,
        workspace: &'a SelectedWorkspace,
        prepared: PreparedTool,
        cancellation: &'a CancellationToken,
        shutdown: &'a CancellationToken,
    ) -> ProfileFuture<'a>;
}

/// Standard adapter over the public `workspace-agent-tools` API.
#[derive(Clone, Debug)]
pub struct WorkspaceAgentProfile {
    tools: WorkspaceTools,
}

impl WorkspaceAgentProfile {
    /// Creates a profile restricted to the configured canonical workspace roots.
    #[must_use]
    pub fn new(workspaces: ConfiguredWorkspaces) -> Self {
        Self {
            tools: WorkspaceTools::new(workspaces),
        }
    }

    /// Returns the underlying public tool executor.
    #[must_use]
    pub fn tools(&self) -> &WorkspaceTools {
        &self.tools
    }
}

impl AgentProfile for WorkspaceAgentProfile {
    fn resource_workspaces(&self) -> Option<&ConfiguredWorkspaces> {
        Some(self.tools.workspaces())
    }

    fn select_workspace(
        &self,
        requested: Option<Vec<Uri>>,
    ) -> Result<SelectedWorkspace, ProfileError> {
        self.tools.select(requested).map_err(ProfileError::from)
    }

    fn server_tools(&self) -> Vec<ToolDefinition> {
        self.tools.definitions()
    }

    fn model_tools(&self) -> Vec<ModelToolDefinition> {
        self.tools
            .definitions()
            .into_iter()
            .filter_map(|tool| {
                Some(ModelToolDefinition::new(
                    tool.name,
                    tool.description?,
                    tool.input_schema?,
                ))
            })
            .collect()
    }

    fn preflight(
        &self,
        workspace: &SelectedWorkspace,
        invocation: ToolInvocation,
    ) -> Result<PreparedTool, ToolError> {
        self.tools.preflight(workspace, invocation)
    }

    fn exact_command_approval(&self, prepared: &PreparedTool) -> Option<ExactCommandApproval> {
        prepared.shell().map(|shell| shell.approval_key().clone())
    }

    fn execute<'a>(
        &'a self,
        workspace: &'a SelectedWorkspace,
        prepared: PreparedTool,
        cancellation: &'a CancellationToken,
        shutdown: &'a CancellationToken,
    ) -> ProfileFuture<'a> {
        Box::pin(
            self.tools
                .execute(workspace, prepared, cancellation, shutdown),
        )
    }
}
