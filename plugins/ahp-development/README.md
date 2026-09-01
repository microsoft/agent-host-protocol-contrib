# AHP Development Plugin

Four focused skills help engineers work on [Agent Host Protocol
(AHP)](https://github.com/microsoft/agent-host-protocol) without treating every
task as the same kind of protocol work.

| Skill | Use when |
| --- | --- |
| `design-ahp-protocol` | Evolving or reviewing wire contracts, reducers, schemas, compatibility, or versioning. |
| `build-ahp-host` | Implementing or reviewing a host, transport, persistence, model adapter, or server lifecycle. |
| `build-ahp-client` | Building an SDK-backed client, state store, reconnection flow, or UI projection. |
| `test-ahp-conformance` | Creating or reviewing black-box protocol tests, fixtures, reports, and interoperability coverage. |

The skills are discovered by Agent Plugins v1.0.0 from immediate child
directories of `skills/`. Install or expose this directory as a plugin root in
an Agent Plugins-compatible client; the client reads `plugin.json` and then
discovers the four `SKILL.md` files.

This initial package supplies skills only: it has no MCP server, client
extension, scripts, or components. To add a skill, create one immediate
`skills/<lowercase-hyphen-name>/SKILL.md` directory, keep its frontmatter name
equal to the directory name, and give its description a distinct activation
boundary so it does not overlap the existing four perspectives.

The skills direct agents to the current protocol source, schemas, and released
documentation in the workspace when available, or to the upstream repository
when it is not. They deliberately distinguish AHP requirements from choices in
individual samples or hosts.

**Audience:** engineers installing reusable AHP development guidance in an
Agent Plugins-compatible client. Start with `plugin.json`, choose the matching
`skills/*/SKILL.md`, then follow the repository
[reading path](../../docs/reading-path.md) for the runnable samples and crates.
