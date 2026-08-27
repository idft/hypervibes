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

## Tools

| Tool | Description | Availability |
| --- | --- | --- |
| `hypervibes_coding_validate_candidate` | Validate candidate analysis code before it can be used. | Sub-agents: analysis coding |
| `hypervibes_coding_submit_report` | Submit the analysis coding result after validation. | Sub-agents: analysis coding |
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
| `hypervibes_send_notification` | Queue a notification for the agent's messaging gateway. A successful call records the notification but does not guarantee delivery. | Operational sub-agents and Chat |

Tools operate within the current agent's account and data boundaries. The
MCP server does not receive the user's Hyperliquid private key.
