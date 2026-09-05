# HyperVibes MCP Server

This directory contains the source for the `hypervibes` Model Context
Protocol (MCP) server that OpenCode agents use to talk to the HyperVibes
backend.

It is **runtime infrastructure, not an agent-authored script**:

- It is installed into the custom OpenCode container image at the fixed
  path `/opt/hypervibes/mcp/`.
- Generated agent workspaces register it as a local stdio MCP server in
  their `opencode.json`.
- The MCP server process is launched **per workspace** by OpenCode, not
  once globally.
- Auth credentials are read from the workspace's own `.env` file in the
  MCP process working directory:
  - `HYPERVIBES_API_BASE_URL`
  - `HYPERVIBES_API_KEY`
  - `HYPERVIBES_AGENT_KEY`

The HyperVibes backend remains the source of truth for agent identity,
memory scoping, instrument allowlists, and order permissions. The MCP
server is a thin transport layer; it does not hold any Hyperliquid private
keys, and its tool schemas never expose `api_key` (or any other credential)
as a callable argument.

## Scheduled Roles

The official scheduled sub-agent roles are Analysis, Trading, and Review.
Analysis uses approved data tools and publishes scoped research memory. Trading
reads that published context and manages orders. Review records outcomes and
learnings and may submit permitted prompt revisions. There is no separate
market-analysis role.

## Strategy Prompts

Chat sessions can review and edit their own per-agent strategy prompts through
`list_strategy_prompts`, `get_strategy_prompt`, and `update_strategy_prompt`.
Prompt responses identify their target by `target_sub_agent_id` and
`target_sub_agent_key`; pass the numeric ID to the get and update tools. These
tools are chat-only: scheduled sub-agents receive their selected strategy prompt
but do not edit it. Updates require the chat operator's one-time OpenCode approval.

## Memories

`write_memory` uses explicit scope: use `scope_kind="agent"` with no targets
for agent-wide records, or `scope_kind="instruments"` with one or more selected
canonical `instrument_ids`. The backend stamps run provenance; callers cannot
supply it. Trading uses `get_trading_context(instrument_id)` to retrieve fresh
analysis evidence and analyst status, then writes `trading_decision` records.

## Operator Logs

The MCP server redirects its stderr to the container log while leaving stdout
exclusively for the MCP stdio protocol. Entries include safe request
parameters, HTTP status, and response shape, but never API keys,
authorization headers, request bodies, or memory content.

If you are an agent reading this file from inside a generated workspace:
do not edit, import, or invoke this module directly. Use the `hypervibes`
MCP tools instead.
