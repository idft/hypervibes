---
slug: /mcp-tools
---

# Agent tools

Tools determine what the main conversation or a sub-agent can access.
Open its configuration under Sub-Agents and choose Edit tools.

## Available capabilities

Depending on the agent type, you can enable tools to:

- Read market prices, candles, positions, and orders.
- Research account history and save supporting material.
- Run indicators or develop custom indicators.
- Read and publish memories, record trade decisions, or update insights.
- Find sub-agents, delegate work, and inspect run results.
- Read chart layouts and edit annotations in the main conversation.
- Send notifications.
- Submit or cancel orders on the assigned Hyperliquid account.

The tool picker shows the options available to each agent. Give workers only
the tools their jobs need. Instructions alone do not prevent a worker from
using a tool you have enabled.

## Trading tools

Order tools act on real funds. Submitting an order does not mean it has filled.
Canceling orders does not close an existing position and can remove protective
stops.

Trading tools do not bypass your [risk settings](Settings.md). Keep order tools
disabled for workers that should only research or review.
