---
slug: /mcp-tools
---

# MCP Tools

HyperVibes provides custom MCP tools that let agents inspect account data,
read and write memory, review strategy context, and manage orders.

## Availability

Tools are available to scheduled jobs, Chat, or both. Job access varies by job
type: each job receives only the tools needed for its role. Chat read tools are
available by default; order actions, memory writes, and prompt updates depend
on the conversation's permissions and may require confirmation.

## Tools

| Tool | Description | Availability |
| --- | --- | --- |
| `hypervibes_coding_validate_candidate` | Validate candidate analysis code before it can be used. | Jobs: analysis coding |
| `hypervibes_coding_submit_report` | Submit the analysis coding result after validation. | Jobs: analysis coding |
| `hypervibes_get_account` | Read the agent's current Hyperliquid account snapshot. | Jobs and Chat |
| `hypervibes_list_strategy_prompts` | List the agent's strategy prompts for review. | Chat only |
| `hypervibes_get_strategy_prompt` | Read one strategy prompt. | Chat only |
| `hypervibes_update_strategy_prompt` | Update one strategy prompt. | Chat only; confirmation required |
| `hypervibes_get_latest_analysis` | Read recent analysis memories for a market. | Jobs: market analysis; Chat |
| `hypervibes_get_market_analysis` | Read the latest market-analysis handoff for a market. | Jobs: trading; Chat |
| `hypervibes_get_memory_detail` | Read one memory and its links. | Jobs: daily review and analysis coding; Chat |
| `hypervibes_list_memories` | List memories visible to the agent. | Jobs: analysis, daily review, and analysis coding; Chat |
| `hypervibes_list_orders` | List orders visible to the agent. | Jobs: trading, daily review, and analysis coding; Chat |
| `hypervibes_list_account_transactions` | Read fills, funding, and ledger events for the account. | Jobs: daily review; Chat |
| `hypervibes_get_order` | Read one order and optionally its event history. | Jobs: trading, daily review, and analysis coding; Chat |
| `hypervibes_write_memory` | Save a memory for the agent. | Jobs: analysis, market analysis, and daily review; Chat permissions apply |
| `hypervibes_submit_orders` | Submit one or more orders through HyperVibes. | Jobs: trading; Chat permissions apply |
| `hypervibes_cancel_orders` | Cancel selected orders. | Jobs: trading; Chat permissions apply |
| `hypervibes_cancel_all_orders` | Cancel all open orders, optionally for one market. | Jobs: trading; Chat permissions apply |

Tools operate within the current agent's account and data boundaries. The
MCP server does not receive the user's Hyperliquid private key.
