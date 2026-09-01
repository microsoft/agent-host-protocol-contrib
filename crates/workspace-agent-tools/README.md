# workspace-agent-tools

Reusable, environment-facing tools for an Agent Host Protocol (AHP) host:

- workspace-root selection and file-URI validation;
- confined `replace_string`;
- bounded `run_shell` execution; and
- AHP tool definitions and `ToolCallResult` conversion.

## Security model

`ConfiguredWorkspaces` canonicalizes configured, existing directory roots. A selected
workspace must be a canonical `file:` URI for an existing directory beneath one of
those roots. `replace_string` accepts only a non-empty relative path, resolves it
canonically, rejects escapes (including symlink escapes), requires a regular UTF-8
file of at most 1 MiB, and replaces exactly one occurrence through a same-directory
temporary file and rename.

`run_shell` uses the selected workspace only as its current working directory. **It
is not an OS sandbox**: a command can read or write paths outside that directory and
may access the network. It has no stdin, bounds combined stdout/stderr capture to
64 KiB, and uses `command-group` process groups (Unix) or Job Objects (Windows) so
timeouts, cancellation, shutdown, and dropped execution futures terminate the whole
process tree.

Approvals are deliberately not stored here. `PreparedShell::approval_key()` is an
`ExactCommandApproval` containing the exact trimmed command. Hosts must persist and
compare that key directly; an executable display name is never an approval identity.

## Typical integration

```rust,no_run
use workspace_agent_tools::{ConfiguredWorkspaces, ToolInvocation, WorkspaceTools};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let tools = WorkspaceTools::new(ConfiguredWorkspaces::from_current_dir()?);
let workspace = tools.select(None)?;
let prepared = tools.preflight(
    &workspace,
    ToolInvocation::run_shell(r#"{"command":"cargo test"}"#),
)?;

if let Some(shell) = prepared.shell() {
    // Ask the user or consult host-owned policy using shell.approval_key().
    let _display_name = shell.display_executable();
}
# Ok(())
# }
```

The host owns AHP lifecycle actions, pending confirmation waiters, policy storage,
and the decision to call `execute`.

**Audience:** host authors adding constrained workspace file and shell tools.
Start at `src/lib.rs`, then read `workspace.rs`, `replace_string.rs`, and
`shell.rs`. Next, read the
[advanced host guide](../../samples/rust/advanced-workspace-host/README.md) for
the confirmation lifecycle and the [architecture](../../docs/architecture.md)
for the approved-shell sequence.
