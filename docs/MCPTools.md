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
| `hypervibes_update_strategy_prompt` | Chat only; Strategy prompts permission (Confirm by default) |
| `hypervibes_submit_prompt_revision` | Review with the prompt-update capability; Trading and Analysis targets only |
| `hypervibes_get_trading_context` | Trading and Chat |
| `hypervibes_get_memory_detail` | Review and Chat |
| `hypervibes_list_memories` | Analysis, Review, and Chat |
| `hypervibes_write_memory` | Analysis, Trading, Review, and permitted Chat |
| `hypervibes_list_orders` | Trading, Review, and Chat |
| `hypervibes_list_account_transactions` | Review and Chat |
| `hypervibes_list_account_trades` | Review and Chat |
| `hypervibes_get_account_trade` | Review and Chat |
| `hypervibes_list_journal_notes` | Review and Chat |
| `hypervibes_add_journal_note` | Review; Chat when Trade notes is set to Allow |
| `hypervibes_get_order` | Trading, Review, and Chat |
| `hypervibes_submit_orders` | Trading and permitted Chat |
| `hypervibes_cancel_orders` | Trading and permitted Chat |
| `hypervibes_cancel_all_orders` | Trading and permitted Chat |
| `hypervibes_send_notification` | Trading and Review by default; other scheduled roles when granted the capability |
| `hypervibes_list_analysis_instruments` | Analysis, Review, and Chat |
| `hypervibes_list_trading_instruments` | Analysis with the Trading-instrument capability |
| `hypervibes_set_trading_instrument_enabled` | Analysis with the Trading-instrument capability |
| `hypervibes_list_indicators` | Analysis, Review, and Chat |
| `hypervibes_get_indicator` | Analysis, Review, and Chat |
| `hypervibes_get_indicator_results` | Analysis, Review, and Chat |
| `hypervibes_create_indicator` | Review with the indicator-write capability; Chat Indicators permission (Confirm by default) |
| `hypervibes_update_indicator` | Review with the indicator-write capability; Chat Indicators permission (Confirm by default) |

`hypervibes:notification_send` is the named capability for queueing a gateway
notification. Trading and Review enable it by default; Analysis and Chat are
denied by default. Notification provenance and capability schema derive from the
authenticated run or conversation, never model-supplied data.

The trade journal tools let Review and Chat agents inspect trades, their fills,
and existing notes. `hypervibes_add_journal_note` adds a note to a trade or an
individual fill, funding payment, or ledger event. Notes show who wrote them and
cannot be edited or deleted. Review agents can add notes; Chat requires the
conversation's **Trade notes** permission to be set to **Allow**.

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

`hypervibes_list_indicators(limit=20, offset=0)` returns a schema-version-2
discovery envelope: `items`, `total`, `offset`, `returned_count`, `next_offset`,
and `budget_reduced`. Follow `next_offset`; do not infer catalog completion from
the requested limit. Items include compact run headers only, with no histories,
source or diagnostics. A `latest_run` is one instrument/timeframe, so inspect
all relevant frozen targets explicitly.

`hypervibes_get_indicator_results` returns a list of schema-version-2 runs,
defaulting to one run, its latest 20 numeric bars and newest 20 marker events.
`bar_limit` and `marker_limit` accept 1–100 as maximum counts. The entire
serialized call is limited to 24 KiB UTF-8 text and 1,000 lines; complete units
may be reduced with honest actual counts/cursors and `budget_reduced` flags.
If even the minimum useful representation cannot fit, the tool asks for a
smaller run `limit`, an exact `run_id`, or operator inspection of an oversized
unit; it does not silently omit runs.

Supply the exact returned `run_id` with every continuation offset. Numeric
pages are independent of marker pages: use `bar_start=next_bar_start` forward,
`bar_end=previous_bar_end` backward (exclusive end; do not combine them), and
`marker_start=next_marker_start` for older events. Markers are newest source
candle first, then original `event_position`. `opened_at` is source candle open
time, not close/confirmation time, and visual `offset` does not change it.
Bars and markers also carry `closed_at`, derived from that exact run's timeframe;
it is a candle boundary, not proof of signal confirmation or execution time.
Bars include stored OHLCV fields when available alongside plots and bar indices.
The exact run's `scheduled_for` is the evidence boundary. Added fields share the
existing whole-call budget; consumers must continue to use actual counts/cursors.
Check `evidence_available`, total/returned counts, `bars_complete` and
`markers_complete`. Unavailable evidence is not absence of signals, and partial
pages cannot establish complete-history claims. Unexpected OpenCode truncation
must be recovered through smaller authorized MCP pages, with coverage limits
recorded, never through the global output-cache filesystem path. See
[Indicators](Indicators.md) for the full coverage and immutable-history contract.

## Memory tools

`hypervibes_write_memory` accepts `scope_kind` of `agent` or `instruments`.
Agent scope requires no instrument IDs; instrument scope requires one or more
unique selected canonical instrument IDs. The server stamps source-run
provenance and never accepts source run or sub-agent identity from metadata.
Use optional `metadata.handoff_version=1` for compact research handoffs and
`links[*].link_type="corrects"` for corrections to exact originals. Every new
Analysis publication receives backend-owned expiry: two immutable source-run
schedule cycles from the memory's actual creation time, including 5m jobs and
omitted memory timeframes, or creation +30m for unscheduled runs. Caller-supplied expiry
metadata is silently ignored for Analysis publications; the MCP tool forwards
metadata and the backend stamps the actual deadline. Price/event invalidation
and cancellation conditions remain part of research. See
[Memory](Memory.md) for historical and correction-expiry behavior.

Invalid/missing/foreign memory links return HTTP 422 with a bounded message,
`code="invalid_memory_link"`, and zero-based `link_index`. Missing and foreign IDs
are indistinguishable; the transaction rolls back completely. Invalid selected
instrument targets return 422 with `code="invalid_memory_target"` and
`instrument_index`. Malformed UUIDs are JSON validation errors; database failures
remain HTTP 500. The MCP error preserves the indexed API response.

Trading copies IDs from context unchanged. For a definitive invalid-link 422,
refetch authorized context, repair only the bad reference, and retry at most once.
Never guess IDs, omit required links, or retry ambiguous transport/500 failures.
Memory writes have no idempotency key and may have committed before an ambiguous
failure. No broader Trading memory-read permission is added.

`hypervibes_get_trading_context(instrument_id)` returns the latest record per
`(Analysis producer, memory type, scope)`, including agent-scoped records and
records targeting that instrument. Corrections are selected per producer/scope/
structured target set instead of correction type, and carry
`correction_target_ids`. Fresh records come first with full content,
summary, and metadata. Stale or disabled records include only ID, provenance,
type, scope and targets, timeframe, expiry, and status; their research bodies
are omitted to keep the response small. The evidence status is `Fresh`,
`Stale`, `Disabled`, or `Superseded`. Corrections become Superseded when none of
their originals remain in current context, and cannot extend an original's
expiry. Legacy prose-only corrections retain existing behavior. Evidence also
includes its server-owned `source_run_id`. A producer without records does not appear in the
evidence array.

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
