This directory is the source of truth for generated OpenCode agent workspaces.

The repository root `.opencode/` directory is developer configuration for
working on this repo. It is not the runtime profile for trading agents.

Generated agent workspaces are mounted at `/workspaces/agents/<agent_key>` from
OpenCode's perspective.

Backend-generated files such as `AGENTS.md`, `opencode.json`, `.env`, and
`generated/agent.json` may be overwritten on regeneration.

User-authored files under `scripts/user/` and `scratch/` are preserved.

The OpenCode container's global runtime config is stored separately at
`agent-runtime/opencode/container/opencode.jsonc` and bind-mounted into the
container. Plugin configuration belongs there, not in generated workspaces.
