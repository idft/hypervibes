---
slug: /mcp-tools
---

# MCP Tools

HyperVibes MCP tools let authorized roles inspect account data, use memory,
manage orders, indicators, revise prompts, and send gateway notifications. Tools are always
scoped to the authenticated agent; the MCP server never receives a Hyperliquid
private key.

## Role availability

| Tool | Availability |
| --- | --- |
| `hypervibes_get_account` | Analysis, Trading, and Chat |
| `hypervibes_list_strategy_prompts` | Chat and Review |
| `hypervibes_get_strategy_prompt` | Chat and Review |
| `hypervibes_update_strategy_prompt` | Chat only; confirmation required |
| `hypervibes_submit_prompt_revision` | Review with the prompt-update capability; Trading and Analysis targets only |
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
| `hypervibes_list_analysis_instruments` | Analysis, Review, and Chat |
| `hypervibes_list_trading_instruments` | Analysis with the Trading-instrument capability |
| `hypervibes_set_trading_instrument_enabled` | Analysis with the Trading-instrument capability |
| `hypervibes_list_indicators` | Analysis, Review, and Chat |
| `hypervibes_get_indicator` | Analysis, Review, and Chat |
| `hypervibes_get_indicator_results` | Analysis, Review, and Chat |
| `hypervibes_create_indicator` | Review with the indicator-write capability; Chat with confirmation |
| `hypervibes_update_indicator` | Review with the indicator-write capability; Chat with confirmation |

`hypervibes:notification_send` is the named capability for queueing a gateway
notification. Trading enables it by default; other scheduled roles and Chat are
denied by default. Notification provenance and capability schema derive from the
authenticated run or conversation, never model-supplied data.

`hypervibes:trading_instrument_write` lets an Analysis run list and atomically
change the Trading allowlist. It can enable only an active instrument currently
selected for Analysis, but may remove any current Trading instrument. Removing
the final instrument pauses new exposure and does not close positions.

Before creating or updating an indicator, callers must obtain target IDs from
`hypervibes_list_analysis_instruments` and pass them unchanged. An empty result
means the operator must select analysis instruments in Settings; callers must
not guess IDs or issue a mutation.

Indicator mutations accept `timeframes`, a list of one to eight unique values
such as `["15m", "1h", "4h"]`; there is no singular `timeframe` compatibility
field. One Pine source and one input-value set execute independently for every
configured timeframe. Use separate definitions for timeframe-specific inputs.
Result reads require one timeframe so evidence from different candle series is
never combined accidentally. They also accept an optional `run_id` for exact
historical provenance. Analysis run credentials are restricted to exact runs in
their frozen dependency snapshot; a dependency frozen as `timed_out` is not
exposed if it completes later. Chat and Review retain bounded historical access.

Chat and indicator-write-enabled Review agents load the `pine-indicators` skill
before authoring or revising Pine source. Indicator read tools expose numeric
plots and an agent-facing `markers` collection. The versioned `visual_data`
storage envelope remains an internal API and persistence contract and is not
returned by MCP tools.

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

## Trading order inputs

`hypervibes_submit_orders` accepts an `orders` array with strict item fields.
A limit order requires `symbol`, `side` (`buy` or `sell`), `order_type` set to
`limit`, positive `size`, and positive `price`. It accepts optional lowercase
`time_in_force` (`gtc`, `ioc`, or `alo`), `reduce_only`, take-profit and
stop-loss arrays, and `memory_record_ids`. For example:

```json
{
  "orders": [{
    "symbol": "ETH",
    "side": "buy",
    "order_type": "limit",
    "size": 0.004,
    "price": 2606,
    "time_in_force": "gtc",
    "memory_record_ids": ["trading-decision-id"]
  }]
}
```

`hypervibes_cancel_orders` requires a `symbol` and numeric exchange `oid` for
each item. It does not accept the Hyperliquid aliases `coin`, `sz`, `limit_px`,
or `tif`.

## Analysis research

Analysis uses approved data tools for research and publishes scoped memories
with `hypervibes_write_memory`. Its MCP surface does not provide reusable code.
It may read published indicator definitions and results, but it cannot change
them. Review with its indicator-write capability may create a definition or
immutable new version from run-scoped evidence. Trading has no indicator tools.
