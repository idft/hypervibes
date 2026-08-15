# Workspace Controller Deployment

Set `WORKSPACE_CONTROL_API_KEY` for both the HyperVibes and OpenCode services. In production,
the controller is private to the Compose network at `http://opencode:14097`; it has no host port.
The development stack publishes it only at `127.0.0.1:${WORKSPACE_CONTROL_PORT:-14097}`. Do not
expose it publicly or place the key in an agent workspace, prompt, MCP configuration, or agent
`.env`.

The `agent_workspaces` named volume holds generated workspaces and coding versions. Back it up
with Podman volume tooling. Preserve `opencode_data` independently because it contains global
OpenCode provider credentials and OAuth state. Back up the production `podman-compose.yaml` with
the database because it holds the agent-encryption key.

Rollout uses a fresh `agent_workspaces` volume: back up the database, retain the legacy host
`workspaces/` directory outside normal operation, deploy, then recreate required agents. The
application intentionally does not copy or remove legacy workspace data.
