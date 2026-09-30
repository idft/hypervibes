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

## Indicator evidence

Indicator discovery/results use compact MCP `schema_version=2` projections;
the HTTP/chart records remain complete and immutable. `list_indicators`
returns an envelope with `items`, `total`, `offset`, `returned_count`, and
`next_offset` (default limit 20), embedding only latest-run headers. Each header
represents one instrument/timeframe, not every frozen target.

`get_indicator_results` defaults to one run, the latest 20 numeric bars and
newest 20 marker events. Both requested page limits accept 1–100, but fitting
may reduce complete units. Every continuation supplies the exact returned
`run_id`: `bar_start=next_bar_start` advances numeric history oldest-first,
`bar_end=previous_bar_end` traverses backward using an exclusive end, and
`marker_start=next_marker_start` independently retrieves older events. Markers
are newest source candle first, with original `event_position` breaking ties.
Source `opened_at` is candle open time; visual offsets never imply an event or
confirmation time. Inspect availability, counts and `bars_complete` /
`markers_complete`; unavailable and partial evidence cannot establish absence
of signals. Unexpected truncation requires a smaller authorized MCP read,
never a global output-cache filesystem read. See `docs/Indicators.md`.

Complete discovery/result calls fit a **24 KiB UTF-8 / 1,000-line** text budget,
including every run and text-block separator. Fitting preserves each requested
range's first unit, or its newest bar for latest/backward numeric reads.
Oversized semantic units and aggregate minimums fail explicitly.

## Testing

`_indicator_text` mirrors the pinned FastMCP text serialization and OpenCode's
joining of text blocks with two newlines. `test_indicator_transport.py` checks
this against the real low-level MCP response, round-tripped through transport
JSON, including structured content. Regression coverage includes dense numeric
and marker histories, complete pagination, changing page sizes, duplicate
events, Unicode, discovery, malformed/missing/failed data, and bounded errors.

Run `python agent-runtime/mcp/test_server.py` from the repository root. It keeps
lightweight fake-MCP unit tests and runs the real transport checks in a child
process when the pinned requirements are installed; otherwise it explicitly
skips that integration check. With the requirements installed, the integration
file can also be run directly.

## Operator Logs

The MCP server redirects its stderr to the container log while leaving stdout
exclusively for the MCP stdio protocol. Entries include safe request
parameters, HTTP status, and response shape, but never API keys,
authorization headers, request bodies, or memory content.
Indicator output diagnostics additionally log tool/schema version, UTF-8 byte
and line counts, returned bar/marker/definition counts, and budget reduction,
without logging evidence content.

If you are an agent reading this file from inside a generated workspace:
do not edit, import, or invoke this module directly. Use the `hypervibes`
MCP tools instead.
