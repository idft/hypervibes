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

## Vibetrading MCP Server

OpenCode agents interact with the Vibetrading backend exclusively through
the `vibetrading` MCP server. There is no workspace-local Python API client
anymore.

The MCP server source lives in `mcp/`. Its Python dependencies are listed in
`mcp/requirements.txt`. Both are baked into the custom OpenCode image at
`/opt/vibetrading/mcp/` by `containers/opencode/Dockerfile`.

The per-agent `opencode.json` registers the MCP server as a local stdio
process. Each generated workspace runs its own MCP process; the process
reads that workspace's `.env` to authenticate against the Vibetrading
backend with the agent's bearer token.

If you are editing this directory:

- Do not reintroduce a workspace-local Python client under
  `vibetrading/py/`. The MCP server is the only Vibetrading API path.
- Do not move the dotenv plugin configuration out of
  `container/opencode.jsonc` into per-workspace configs; global plugins
  belong in the container-global config.
