# Vibetrading MCP Server

This directory contains the source for the `vibetrading` Model Context
Protocol (MCP) server that OpenCode agents use to talk to the Vibetrading
backend.

It is **runtime infrastructure, not an agent-authored script**:

- It is installed into the custom OpenCode container image at the fixed
  path `/opt/vibetrading/mcp/`.
- Generated agent workspaces register it as a local stdio MCP server in
  their `opencode.json`.
- The MCP server process is launched **per workspace** by OpenCode, not
  once globally.
- Auth credentials are read from the workspace's own `.env` file:
  - `VIBETRADING_API_BASE_URL`
  - `VIBETRADING_API_KEY`
  - `VIBETRADING_AGENT_KEY`

The Vibetrading backend remains the source of truth for agent identity,
memory scoping, instrument allowlists, and order permissions. The MCP
server is a thin transport layer; it does not hold any Hyperliquid private
keys, and its tool schemas never expose `api_key` (or any other credential)
as a callable argument.

If you are an agent reading this file from inside a generated workspace:
do not edit, import, or invoke this module directly. Use the `vibetrading`
MCP tools instead.
