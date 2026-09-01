//! Workspace-confined, environment-facing tools for AHP host integrations.
//!
//! Shell commands use a selected workspace as their current directory, but are not
//! sandboxed. Hosts own lifecycle actions, confirmation waiters, and approval storage.

mod definitions;
mod executor;
mod output;
mod replace_string;
mod result;
mod shell;
mod workspace;

pub use definitions::tool_definitions;
pub use executor::{
    ExecutionOutcome, PreparedTool, ToolError, ToolExecutor, ToolInvocation, WorkspaceTools,
    REPLACE_STRING_TOOL, RUN_SHELL_TOOL,
};
pub use replace_string::PreparedReplaceString;
pub use replace_string::MAX_REPLACE_FILE_BYTES;
pub use result::ToolExecution;
pub use shell::{
    ExactCommandApproval, PreparedShell, DEFAULT_TIMEOUT_SECONDS, MAX_OUTPUT_BYTES,
    MAX_TIMEOUT_SECONDS,
};
pub use workspace::{
    canonical_file_uri, canonical_file_uri_path, canonical_path, file_uri_path,
    resolve_existing_file, ConfiguredWorkspaces, SelectedWorkspace, WorkspaceError,
};
