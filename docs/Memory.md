---
slug: /concepts/memory
---

# Memory System

Memory is time-ordered, agent-owned context. Every record has server-owned
source-run provenance and an explicit scope: `agent` or `instruments`.

## Scope and targets

Agent-scoped records apply to the whole agent and have no instrument targets.
Instrument-scoped records target one or more selected canonical Hyperliquid
instruments. Historical targets remain attached if an instrument is later
deselected. Links record evidence relationships without crossing agent
ownership boundaries.

## Analysis research

Analysis jobs may write any valid non-reserved memory type. Their output is
discoverable rather than mapped to an analyst and can be agent-scoped or target
one or more instruments. Analysis owns the accuracy of its quoted values,
timestamps, and interpretations; the backend validates storage shape, ownership,
and references, not factual correctness.

Every new Analysis publication receives a backend-owned
`metadata.stale_after` and `expiry_policy="analysis_schedule_v3"`. Expiry is two
cycles of the **immutable producing run's schedule**, anchored to the memory's
actual creation time (5m => creation + 10m, 15m => +30m, 4h => +8h). The memory's
optional evidence timeframe does not override that schedule; omitted timeframes
work the same way. Unscheduled producers use creation time +30m. Queue delays and
runtime/model latency do not consume the memory's validity period before it is
created. Stored expiry is shared by the API, Trading context, and operator UI.

Analysis cannot shorten or extend memory expiry. Supplied `metadata.stale_after`
and `expiry_policy` are overwritten and `valid_for_seconds` is removed, even when
their values are malformed. The MCP tool forwards metadata without rejecting
expiry controls; the backend stamps the actual deadline before validation, and
the memory write response includes it as `expires_at`. Prose deadlines have no
storage effect. Price/event invalidation and cancellation conditions remain part
of research: a fresh memory does not guarantee an actionable entry.

Historical records are not rewritten, including prior caller-set validity and
`analysis_schedule_v1` and scheduled-boundary `analysis_schedule_v2` deadlines.
Explicit `stale_after` takes precedence over
`valid_for_seconds`, which is relative to creation. Without explicit validity,
legacy provenance-bearing research uses its **memory timeframe** (15m => 30m, 1h => 2h,
1d => 48h, other/5m/omitted => 30m), relative to creation. Records without run
provenance have no implicit expiry. Non-Analysis writes retain their existing
explicit-validity behavior and validation. This preserves historical freshness
claims.

Trading reads the latest record per `(Analysis producer, memory type, scope)`
for the requested instrument, including agent-wide records. Fresh records are
returned first with their complete research; expired or disabled records carry
only their identity, provenance, expiry, and status so historical research
cannot bury fresh evidence in a large tool response. Missing or failed runs do
not appear as evidence records. Missing, failed, disabled, and stale research
does not block the scheduler or order gateway.

### Handoffs and corrections

Prefer optional `metadata.handoff_version=1` and a compact handoff separating
source observations from interpretation: exact evidence/run IDs, instrument,
timeframe and boundary, source bars (`bar_index`, `opened_at`, derived `closed_at`),
quoted numeric observations, bias, setup status, applicable entry/stop/targets,
confidence rationale, and price/event invalidation and cancellation conditions.
This is an agent-side formatting convention; legacy prose-only memories remain
accepted and readable.
Qualitative confidence is not a calibrated probability.

For example, a versioned handoff can keep short prose in `content` and use this
optional metadata layout (keys are conventions, not factual-validation rules):

```json
{
  "handoff_version": 1,
  "evidence_boundary": "2026-09-30T12:00:00Z",
  "sources": [{
    "indicator_run_id": "exact UUID copied from the tool response",
    "instrument_id": "ETH",
    "timeframe": "5m",
    "bar_index": 499,
    "opened_at": "2026-09-30T11:55:00Z",
    "closed_at": "2026-09-30T12:00:00Z"
  }],
  "observations": {"fast_ema": 2689.21, "slow_ema": 2691.10},
  "interpretation": {
    "bias": "bearish",
    "setup_status": "no_setup",
    "confidence": {"label": "moderate", "rationale": "EMA alignment without entry confirmation"},
    "invalidation": "A new closed-bar bullish crossover"
  }
}
```

Actionable handoffs include entry, stop, and target observations/interpretation
as applicable; unavailable sources are labeled explicitly. IDs in a real
publication must be copied from authorized evidence, never from this example.

Reuse stable handoff types. Corrections use an outgoing `link_type="corrects"`
to the exact original memory; Analysis does not choose their expiry either.
Trading context returns `correction_target_ids` and the latest correction per
producer/scope/target set, even if the correction type name changes. A correction
can be Fresh only while its original remains current, and its context expiry is
capped at the original's expiry. Once all originals leave current context,
the correction is `Superseded`, with no research body. Originals are not silently
modified or deleted. Durable data-quality guidance belongs in a separate memory
without an entry-correction link; disagreements between producers remain visible.
Legacy prose-only/ad hoc corrections have no machine-resolvable relation and keep
their existing selection/freshness behavior.

### Invalid references and recovery

Memory insertion, instrument targets, and links are transactional. A nonexistent
or foreign target produces the same bounded HTTP 422 JSON response:

```json
{
  "error": "links[1].target_memory_id is unavailable; copy the exact ID from authorized same-agent context and retry once",
  "code": "invalid_memory_link",
  "link_index": 1
}
```

No partial record or link is retained. Unselected/inactive instrument targets
return 422 with `code="invalid_memory_target"` and a zero-based `instrument_index`.
Malformed UUIDs return a JSON validation error; genuine database failures remain
generic HTTP 500s. Classification uses typed store errors, not string matching.

Trading copies IDs unchanged from context. On a definitive invalid-link 422,
it refetches authorized instrument context, repairs only the identified bad
reference, and retries at most once. It must not guess UUIDs or drop required
evidence. Memory writes have no idempotency key; timeouts, transport failures,
500s, and malformed success responses may follow a committed write. Trading
reports uncertainty rather than blindly retrying and retains its restricted
context-only read access.

## Trading decisions

Trading writes the reserved `trading_decision` type as a durable audit and UI
log. It should cover every evaluated instrument, including no-trade and
position-management outcomes, and link all considered evidence. Runtime
instructions require a successfully returned decision ID in opening orders'
`memory_record_ids`; if publication fails, Trading avoids new exposure and
continues reduce-only protection/risk management. This is an agent instruction,
not an additional order-gateway validation gate. Missing/uncertain decision IDs
must be reported per instrument.

## Review and learnings

Review records outcomes and learnings. `agent_learnings` preserve durable
lessons for future Analysis, Trading, and Review runs. Framework-owned types are
reserved: Trading writes `trading_decision`, and Review writes review and
learning records.

## Operator browser

The agent's Memories tab discovers its type list from stored records, including
custom Analysis types. Selecting a type filters records before pagination. The
time filter offers rolling 1h, 6h, and 24h windows or an inclusive custom
range of local calendar dates (including daylight-saving transitions). The page
sends the browser's time zone with a custom range and keeps the selected type
and time filter while loading older records or receiving live updates. When a
time filter is active, the type list shows only types with records in that window.
