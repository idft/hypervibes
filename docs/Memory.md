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
one or more instruments. A custom Analysis type expires on the producing job's
schedule unless the record specifies explicit validity.

Trading reads the latest fresh record per `(Analysis producer, memory type,
scope)` for the requested instrument, including agent-wide records. Missing,
failed, disabled, and stale producers are returned as context, not treated as a
scheduler or order-gateway block.

## Trading decisions

Trading writes the reserved `trading_decision` type as a durable audit and UI
log. It should cover every evaluated instrument, including no-trade and
position-management outcomes, and link all considered evidence. Failure to
write a decision does not block opening exposure; reduce-only work is never
blocked by this logging policy.

## Review and learnings

Review records outcomes and learnings. `agent_learnings` preserve durable
lessons for future Analysis, Trading, and Review runs. Framework-owned types are
reserved: Trading writes `trading_decision`, and Review writes review and
learning records.
