---
slug: /mcp-tools
---

# MCP Tools

HyperVibes provides custom MCP tools that let agents inspect account data,
read and write memory, review strategy context, manage orders, and send
notifications through the configured messaging gateway.

## Availability

Tools are available to scheduled sub-agents, Chat, or both. Sub-agent access varies by
type: each sub-agent receives only the tools needed for its role. Chat read tools are
available by default; order actions, memory writes, and prompt updates depend
on the conversation's permissions and may require confirmation.

`hypervibes:notification_send` is the named capability for queueing a gateway
notification. Every scheduled role is eligible for the capability once
per-sub-agent controls are available. Until then, its static default is enabled
only for trading. It is denied for other scheduled roles and for Chat.

When isolated runtime credentials are enabled, notification provenance and the
capability schema are derived from the authenticated run or conversation scope,
not supplied by the model. This provenance never includes a gateway token or
destination and does not affect durable notification delivery after its source
workspace is deleted.

## Tools

| Tool | Description | Availability |
| --- | --- | --- |
| `hypervibes_coding_validate_candidate` | Validate candidate analysis code before it can be used. | Sub-agents: analysis coding |
| `hypervibes_coding_submit_report` | Submit the analysis coding result after validation. | Sub-agents: analysis coding |
| `hypervibes_run_analysis_tool` | Run a manifest-declared quantitative tool with run-local inputs and outputs. The validated output records its package hash, package version, tool ID, and tool version. | Sub-agents: analysis and trading |
| `hypervibes_get_account` | Read the agent's current Hyperliquid account snapshot. | Sub-agents and Chat |
| `hypervibes_list_strategy_prompts` | List the agent's strategy prompts for review. | Chat only |
| `hypervibes_get_strategy_prompt` | Read one strategy prompt. | Chat only |
| `hypervibes_update_strategy_prompt` | Update one strategy prompt. | Chat only; confirmation required |
| `hypervibes_get_latest_analysis` | Read recent analysis memories for a market. | Sub-agents: market analysis; Chat |
| `hypervibes_get_market_analysis` | Read the latest market-analysis handoff for a market. | Sub-agents: trading; Chat |
| `hypervibes_get_memory_detail` | Read one memory and its links. | Sub-agents: daily review and analysis coding; Chat |
| `hypervibes_list_memories` | List memories visible to the agent. | Sub-agents: analysis, daily review, and analysis coding; Chat |
| `hypervibes_list_orders` | List orders visible to the agent. | Sub-agents: trading, daily review, and analysis coding; Chat |
| `hypervibes_list_account_transactions` | Read fills, funding, and ledger events for the account. | Sub-agents: daily review; Chat |
| `hypervibes_get_order` | Read one order and optionally its event history. | Sub-agents: trading, daily review, and analysis coding; Chat |
| `hypervibes_write_memory` | Save a memory for the agent. | Sub-agents: analysis, market analysis, and daily review; Chat permissions apply |
| `hypervibes_submit_orders` | Submit one or more orders through HyperVibes. | Sub-agents: trading; Chat permissions apply |
| `hypervibes_cancel_orders` | Cancel selected orders. | Sub-agents: trading; Chat permissions apply |
| `hypervibes_cancel_all_orders` | Cancel all open orders, optionally for one market. | Sub-agents: trading; Chat permissions apply |
| `hypervibes_send_notification` | Queue a notification for the agent's messaging gateway. A successful call records the notification but does not guarantee delivery. | Sub-agents: trading only by default |

Tools operate within the current agent's account and data boundaries. The
MCP server does not receive the user's Hyperliquid private key.

## Quantitative Tools

Reusable quantitative packages live under `scripts/user/` and declare tools in
`manifest.json`. The manifest is schema-versioned and identifies each tool's
entrypoint, supported inputs and timeframes, minimum candles, required
arguments, output schema, and version. The legacy `analyze.py` entrypoint is
registered as the `analyze` tool when an existing package has not yet supplied a
manifest.

Agents do not execute package entrypoints directly. The launcher accepts only a
declared tool, limits input and output paths to the current workspace's
`scratch/` tree, applies the immutable analysis runtime and timeout, validates
the output envelope, and atomically records the package binding in that output.
