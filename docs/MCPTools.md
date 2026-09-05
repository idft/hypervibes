---
slug: /mcp-tools
---

# MCP Tools

HyperVibes MCP tools let authorized roles inspect account data, use memory,
manage orders, revise prompts, and send gateway notifications. Tools are always
scoped to the authenticated agent; the MCP server never receives a Hyperliquid
private key.

## Role availability

| Tool | Availability |
| --- | --- |
| `hypervibes_get_account` | Analysis, Trading, and Chat |
| `hypervibes_list_strategy_prompts` | Chat and Review |
| `hypervibes_get_strategy_prompt` | Chat and Review |
| `hypervibes_update_strategy_prompt` | Chat only; confirmation required |
| `hypervibes_submit_prompt_revision` | Review only, for snapshotted permitted targets |
| `hypervibes_get_trading_context` | Trading and Chat |
| `hypervibes_get_memory_detail` | Review and Chat |
| `hypervibes_list_memories` | Analysis, Review, and Chat |
| `hypervibes_write_memory` | Analysis, Trading, Review, and permitted Chat |
| `hypervibes_list_orders` | Trading, Review, and Chat |
| `hypervibes_list_account_transactions` | Review and Chat |
| `hypervibes_get_order` | Trading, Review, and Chat |
| `hypervibes_submit_orders` | Trading and permitted Chat |
| `hypervibes_cancel_orders` | Trading and permitted Chat |
| `hypervibes_cancel_all_orders` | Trading and permitted Chat |
| `hypervibes_send_notification` | Trading by default |

`hypervibes:notification_send` is the named capability for queueing a gateway
notification. Trading enables it by default; other scheduled roles and Chat are
denied by default. Notification provenance and capability schema derive from the
authenticated run or conversation, never model-supplied data.

## Memory tools

`hypervibes_write_memory` accepts `scope_kind` of `agent` or `instruments`.
Agent scope requires no instrument IDs; instrument scope requires one or more
unique selected canonical instrument IDs. The server stamps source-run
provenance and never accepts source run or sub-agent identity from metadata.

`hypervibes_get_trading_context(instrument_id)` returns the latest fresh output
per `(Analysis producer, memory type, scope)`, including agent-scoped records
and records targeting that instrument. It includes evidence provenance, type,
scope and targets, expiry, and producer status: `fresh`, `stale`, `missing`,
`failed`, or `disabled`.

## Analysis research

Analysis uses approved data tools for research and publishes scoped memories
with `hypervibes_write_memory`. Its MCP surface does not provide reusable code.
