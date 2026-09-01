//! AHP definitions for the workspace tools.

use ahp_types::state::{ToolAnnotations, ToolDefinition};
use serde_json::{json, Value};

use crate::{REPLACE_STRING_TOOL, RUN_SHELL_TOOL};

/// Returns the AHP definitions for `replace_string` and `run_shell`.
#[must_use]
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: REPLACE_STRING_TOOL.into(),
            title: Some("Replace text in a workspace file".into()),
            description: Some(
                "Replaces exactly one occurrence of old_string in an existing UTF-8 file beneath the selected workspace."
                    .into(),
            ),
            input_schema: Some(replace_string_schema()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                idempotent_hint: Some(false),
                open_world_hint: Some(false),
                ..ToolAnnotations::default()
            }),
            meta: None,
        },
        ToolDefinition {
            name: RUN_SHELL_TOOL.into(),
            title: Some("Run workspace shell command".into()),
            description: Some(
                "Runs a platform-shell command with the selected workspace as cwd. This scopes cwd only; it is not an OS sandbox and commands may access paths outside the workspace."
                    .into(),
            ),
            input_schema: Some(run_shell_schema()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                idempotent_hint: Some(false),
                open_world_hint: Some(true),
                ..ToolAnnotations::default()
            }),
            meta: None,
        },
    ]
}

fn replace_string_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["path", "old_string", "new_string"],
        "properties": {
            "path": { "type": "string", "description": "Relative UTF-8 file path beneath the selected workspace" },
            "old_string": { "type": "string", "description": "Non-empty text that occurs exactly once" },
            "new_string": { "type": "string" }
        }
    })
}

fn run_shell_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["command"],
        "properties": {
            "command": { "type": "string", "minLength": 1 },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 1,
                "maximum": 300,
                "description": "Optional timeout; defaults to 30 seconds and is capped at 300."
            }
        }
    })
}
