//! Public preparation and execution API for confined workspace tools.

use std::fmt;

use ahp_types::state::ToolDefinition;
use tokio_util::sync::CancellationToken;

use crate::replace_string;
use crate::shell::{self, PreparedShell, ShellExecution};
use crate::{
    tool_definitions, ConfiguredWorkspaces, PreparedReplaceString, SelectedWorkspace,
    ToolExecution, WorkspaceError,
};

/// Registered AHP tool name for exact text replacement.
pub const REPLACE_STRING_TOOL: &str = "replace_string";
/// Registered AHP tool name for platform-shell execution.
pub const RUN_SHELL_TOOL: &str = "run_shell";

/// An untrusted AHP tool name and JSON input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolInvocation {
    name: String,
    input: String,
}

impl ToolInvocation {
    /// Creates an invocation for a named AHP tool.
    #[must_use]
    pub fn new(name: impl Into<String>, input: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            input: input.into(),
        }
    }

    /// Creates a `replace_string` invocation.
    #[must_use]
    pub fn replace_string(input: impl Into<String>) -> Self {
        Self::new(REPLACE_STRING_TOOL, input)
    }

    /// Creates a `run_shell` invocation.
    #[must_use]
    pub fn run_shell(input: impl Into<String>) -> Self {
        Self::new(RUN_SHELL_TOOL, input)
    }

    /// Returns the requested tool name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the original JSON tool input.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }
}

/// Parsed, but not yet executed, tool input.
#[derive(Debug)]
pub enum PreparedTool {
    /// A confined replacement request.
    ReplaceString(PreparedReplaceString),
    /// A shell command whose exact approval key is available before execution.
    RunShell(PreparedShell),
}

impl PreparedTool {
    /// Returns the shell request when this is a `run_shell` invocation.
    #[must_use]
    pub fn shell(&self) -> Option<&PreparedShell> {
        match self {
            Self::RunShell(shell) => Some(shell),
            Self::ReplaceString(_) => None,
        }
    }

    /// Returns the replace request when this is a `replace_string` invocation.
    #[must_use]
    pub fn replace_string(&self) -> Option<&PreparedReplaceString> {
        match self {
            Self::ReplaceString(replace) => Some(replace),
            Self::RunShell(_) => None,
        }
    }
}

/// An execution result, distinguished from lifecycle cancellation.
#[derive(Clone, Debug)]
pub enum ExecutionOutcome {
    /// The tool produced an AHP-convertible result.
    Completed(ToolExecution),
    /// Cancellation or host shutdown prevented completion.
    Cancelled,
}

/// A parsing, selection, or dispatch error before a tool is executed.
#[derive(Debug)]
pub enum ToolError {
    /// Workspace validation failed.
    Workspace(WorkspaceError),
    /// Tool input was not valid for the selected tool.
    InvalidInput(String),
    /// The invocation named a tool that this crate does not provide.
    UnsupportedTool(String),
}

impl ToolError {
    /// Converts a preflight error into an AHP-convertible failed execution.
    #[must_use]
    pub fn into_execution(self) -> ToolExecution {
        ToolExecution::failed(self.to_string())
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace(error) => error.fmt(formatter),
            Self::InvalidInput(error) => formatter.write_str(error),
            Self::UnsupportedTool(name) => write!(formatter, "unsupported workspace tool {name:?}"),
        }
    }
}

impl std::error::Error for ToolError {}

impl From<WorkspaceError> for ToolError {
    fn from(value: WorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

/// Workspace tool definitions, selection, preflight, and execution.
#[derive(Clone, Debug)]
pub struct WorkspaceTools {
    workspaces: ConfiguredWorkspaces,
}

/// Backwards-friendly name for a [`WorkspaceTools`] dispatcher.
pub type ToolExecutor = WorkspaceTools;

impl WorkspaceTools {
    /// Creates a dispatcher limited to `workspaces`.
    #[must_use]
    pub fn new(workspaces: ConfiguredWorkspaces) -> Self {
        Self { workspaces }
    }

    /// Returns the configured workspace selector.
    #[must_use]
    pub fn workspaces(&self) -> &ConfiguredWorkspaces {
        &self.workspaces
    }

    /// Returns this dispatcher's AHP tool definitions.
    #[must_use]
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        tool_definitions()
    }

    /// Selects an allowed workspace from the AHP `workingDirectories` value.
    pub fn select(
        &self,
        requested: Option<Vec<ahp_types::common::Uri>>,
    ) -> Result<SelectedWorkspace, WorkspaceError> {
        self.workspaces.select(requested)
    }

    /// Parses input and validates the selected workspace without running the tool.
    ///
    /// For a shell invocation, inspect `PreparedTool::shell` and compare its exact
    /// approval key with host-owned policy before calling [`Self::execute`].
    pub fn preflight(
        &self,
        workspace: &SelectedWorkspace,
        invocation: ToolInvocation,
    ) -> Result<PreparedTool, ToolError> {
        self.workspaces.revalidate(workspace)?;
        match invocation.name.as_str() {
            REPLACE_STRING_TOOL => replace_string::PreparedReplaceString::parse(&invocation.input)
                .map(PreparedTool::ReplaceString)
                .map_err(ToolError::InvalidInput),
            RUN_SHELL_TOOL => shell::parse(&invocation.input)
                .map(PreparedTool::RunShell)
                .map_err(ToolError::InvalidInput),
            _ => Err(ToolError::UnsupportedTool(invocation.name)),
        }
    }

    /// Executes a preflighted tool after host-owned authorization has completed.
    ///
    /// Both tokens cause running shell process trees to be terminated. Replacement
    /// is synchronous, bounded file I/O and is only started while both tokens live.
    pub async fn execute(
        &self,
        workspace: &SelectedWorkspace,
        prepared: PreparedTool,
        cancellation: &CancellationToken,
        shutdown: &CancellationToken,
    ) -> ExecutionOutcome {
        if cancellation.is_cancelled() || shutdown.is_cancelled() {
            return ExecutionOutcome::Cancelled;
        }
        let workspace = match self.workspaces.revalidate(workspace) {
            Ok(workspace) => workspace,
            Err(error) => {
                return ExecutionOutcome::Completed(ToolExecution::failed(error.to_string()))
            }
        };
        match prepared {
            PreparedTool::ReplaceString(prepared) => {
                ExecutionOutcome::Completed(replace_string::execute(workspace.path(), prepared))
            }
            PreparedTool::RunShell(prepared) => {
                match shell::execute(&prepared, workspace.path(), cancellation, shutdown).await {
                    ShellExecution::Completed(result) => ExecutionOutcome::Completed(result),
                    ShellExecution::Cancelled => ExecutionOutcome::Cancelled,
                }
            }
        }
    }
}
