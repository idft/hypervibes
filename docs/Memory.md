# Memory System

Related docs:

- `README.md` for top-level V2 direction
- `Agents.md` for the agent registry and the mapping from `agent_key` to execution account ownership
- `Architecture.md` for implementation-language and runtime options
- `Hermes.md` for the agent that will consume this system via an Agent Skill

## Goal

The memory system lets trading agents **save** analysis as timestamped records and **retrieve** those records later as time-ordered roll-ups.

The purpose is learning over time: an agent writes its analysis on a schedule, and a later job reads back recent analysis to determine what went right, what went wrong, and what to do next.

This is deliberately a **simple, append-only store**, not a general AI memory framework and not a search engine.

There is **no semantic search**. Retrieval is by `symbol`, `timeframe`, `memory_type`, and time window.

## Core Concept

An agent saves memories. Each memory is:

- owned by one agent (`agent_key`)
- about one instrument (`symbol`)
- optionally anchored to a `timeframe` (e.g. `1m`, `15m`, `1h`, `1d`) — or general (no timeframe)
- tagged with a free-text `memory_type`
- carries a short `summary`, a freeform markdown `content` body, and optional structured `metadata`

The system is used as a **cascading roll-up**: each analysis loop reads the most recent memories from the timeframe directly below it, then writes a new memory at its own timeframe.

Example:

- the `1d` loop reads recent `1h` memories for `BTC`, then writes a `1d` memory
- the `1h` loop reads recent `15m` memories for `BTC`, then writes a `1h` memory
- the `15m` loop reads recent lower-timeframe memories, then writes a `15m` memory

Each analysis loop reads **one specific timeframe** at a time. Separately, execution contexts can use `GET /api/v1/memories/latest` to fetch the current valid memory per timeframe for a symbol and `memory_type`.

## Design Principles

### 1. Append-only and immutable

A memory is a snapshot of what an agent believed at a point in time. It is never updated or rewritten.

If an agent's view changes, it writes a **new** memory at the new time. Because retrieval is time-ordered, the newer memory naturally takes precedence.

There are no `PUT`/`PATCH`/`DELETE` operations in the agent API. This removes update races and preserves the historical record needed for evaluation.

### 2. Evaluation is just another memory

The system does not annotate or score old records in place.

When a daily loop reflects on prior analysis, it writes a **new** memory (for example a `reflection` or `outcome`) whose `content`/`metadata` can reference the prior memories it evaluated (by their `id`). The original records remain untouched.

### 3. One `agent_key` spans all timeframes

A single agent operates across all timeframes. It writes `15m`, `1h`, and `1d` memories, and it performs its own roll-ups by reading its own lower-timeframe memories.

There is no separate agent identity per timeframe-role for V1.

### 4. Agents only access their own memories

The agent API authenticates by API key, which resolves to exactly one `agent_key`. An agent can only read and write its own memories.

There is no cross-agent read access in V1.

### 5. Content-first

The primary payload is the freeform markdown `content` body the agent produces. Optional structured fields go in `metadata` (JSONB). A short `summary` exists purely for UI listing.

This keeps the system flexible: structured fields can be added inside `metadata` without migrations, and frequently-queried keys can be promoted to real columns later.

## Data Model

The memory system lives in its own Postgres schema: `memory`.

It uses a single table for V1: `memory.records`.

### `memory.records`

| Column        | Type                                                              | Notes                                            |
| ------------- | ----------------------------------------------------------------- | ------------------------------------------------ |
| `id`          | `UUID PRIMARY KEY`                                                | server-generated                                 |
| `created_at`  | `TIMESTAMPTZ NOT NULL DEFAULT now()`                              | the memory timestamp; primary sort key           |
| `agent_key`   | `TEXT NOT NULL REFERENCES agents(agent_key) ON DELETE CASCADE` | ownership boundary                         |
| `symbol`      | `TEXT NOT NULL`                                                   | free text, agent decides (e.g. `BTC`)            |
| `timeframe`   | `TEXT` (nullable)                                                 | free text (`1m`/`15m`/`1h`/`1d`); NULL = general |
| `memory_type` | `TEXT NOT NULL`                                                   | free text (e.g. `observation`/`plan`/`reflection`) |
| `summary`     | `TEXT NOT NULL`                                                   | short one-liner for UI lists                     |
| `content`     | `TEXT NOT NULL`                                                   | main freeform markdown body                      |
| `metadata`    | `JSONB NOT NULL DEFAULT '{}'::jsonb`                              | optional structured fields                       |

Field roles:

- **`summary`** — a short human/LLM-readable line shown in UI lists.
- **`content`** — the main analysis, written as markdown, rendered in the UI and fed back to the agent on roll-up.
- **`metadata`** — optional machine-usable structured bits the agent chooses to attach (confidence, levels, targets, referenced memory IDs, etc.). Never required.

## Analysis Memory Metadata Contract

The Hermes `analysis-loop` skill now uses a semi-structured `metadata`
contract for `memory_type="analysis"` records. The backend still stores
`metadata` as free-form JSONB, but the reserved top-level keys below are the
stable handoff that the `trading-loop` skill can depend on.

Recommended shape:

```json
{
  "schema_version": 1,
  "analysis_kind": "trade_setup",
  "symbol": "BTC",
  "timeframe": "15m",
  "generated_at": "2026-06-17T12:15:00Z",
  "valid_for_seconds": 1200,
  "bias": "bullish",
  "confidence": 0.74,
  "entry_setups": [
    {
      "id": "long_pullback_1",
      "side": "buy",
      "order_type": "limit",
      "entry_zone": { "low": "67020", "high": "67110" },
      "size_fraction": "0.33",
      "confidence": 0.72,
      "reason": "Retest of reclaimed intraday support"
    }
  ],
  "take_profit_levels": [
    { "price": "67480", "size_fraction": "0.5" }
  ],
  "stop_loss_levels": [
    { "price": "66880", "kind": "hard_stop" }
  ],
  "invalidation": {
    "type": "price_below",
    "level": "66880"
  },
  "do_not_trade_if": [
    "already_in_position_same_direction",
    "setup_age_minutes_gt_20"
  ],
  "source_memory_ids": [],
  "extensions": {}
}
```

Rules:

- Keep the reserved top-level fields above stable enough for the trading loop
  to consume.
- Put arbitrary strategy-specific additions only under `metadata.extensions`.
- Do not allow random top-level key drift if the field is part of the planned
  analysis-to-trading contract.
- `summary` stays a concise one-liner for UI listing.
- `content` stays the human-readable markdown narrative.

### Proposed Future Improvement: Multi-Model Analysis

This section is an **initial draft plan**, not current implemented behavior.

In the future, the same symbol and timeframe may have multiple
`memory_type="analysis"` memories produced by different LLM models in parallel.
One producer will be the execution-driving "primary" analysis, while other
models act as shadow or benchmark analysts so their performance can be compared
later.

The current recommendation is to keep all of these records as
`memory_type="analysis"` and identify the producer in `metadata`, rather than
splitting models into different memory types.

Proposed metadata addition:

```json
{
  "schema_version": 1,
  "analysis_kind": "trade_setup",
  "symbol": "BTC",
  "timeframe": "15m",
  "generated_at": "2026-06-22T18:15:00Z",
  "valid_for_seconds": 1200,
  "producer": {
    "role": "primary",
    "model": "claude-sonnet-4",
    "provider": "anthropic",
    "job_id": "vibetrading-analysis-primary",
    "run_id": "run_2026_06_22_181500"
  },
  "bias": "bullish",
  "confidence": 0.74,
  "entry_setups": []
}
```

Planned semantics:

- Keep `memory_type="analysis"` for all analysis producers.
- Use `metadata.producer.role` to distinguish `primary`, `shadow`,
  `benchmark`, or other future roles.
- Do not infer the execution-driving memory from the model name alone.
- Keep model provenance inside metadata until a dedicated provenance table is
  justified.

Planned execution behavior:

- The trading loop should eventually read only the `primary` analysis producer
  by default.
- Shadow or benchmark analyses should still be stored so they can be evaluated
  later against price action and actual trade outcomes.
- Trade execution or reflection memories should be able to reference the source
  analysis memory by `id`, and optionally a specific `entry_setup.id`, so model
  performance can be compared after the fact.

Planned API direction:

- The current `GET /api/v1/memories/latest?symbol=BTC&memory_type=analysis`
  endpoint is intentionally simple and does not yet distinguish producers.
- If and when multiple producers are added, a future filter such as
  `producer_role=primary` is the likely extension point.
- Grouping for execution should remain "latest valid per timeframe after
  filtering to the selected producer role."
- Grouping for evaluation may later need to distinguish by timeframe plus
  producer identity.

This is intentionally deferred until there is more than one real analysis
producer in the system. The immediate implementation goal remains: one latest
valid analysis memory per timeframe for the current agent.

Nullability:

- `symbol` is **required**. Every V1 memory is at least about one instrument. Agent-wide/macro memories not tied to an instrument are deferred.
- `timeframe` is **optional**. A memory with a timeframe is candle-anchored analysis; a memory without one is a general/standing memory about the `symbol` + `memory_type`.

### Indexes

- `(agent_key, symbol, timeframe, created_at DESC)` — covers last-N and time-window retrieval per timeframe, including the general (NULL) case.
- `(agent_key, created_at DESC)` — broad per-agent retrieval.

No additional indexes for V1. NULL `timeframe` values index correctly in a Postgres btree.

### Migration

Add `migrations/0003_memory.sql` (and `migrations/0003_memory.down.sql`) following the existing plain-SQL migration pattern:

1. `CREATE SCHEMA IF NOT EXISTS memory;`
2. `CREATE TABLE IF NOT EXISTS memory.records (...)` with the columns above.
3. The two indexes above.
4. Foreign key `agent_key -> agents(agent_key) ON DELETE CASCADE`.

The `.down.sql` drops the table and the schema.

## Agent API

The agent API is JSON and authenticated by API key. It does **not** include an `agent_id` in any path; the API key determines which agent the memories belong to.

All routes are under `/api/v1/memories`.

### Authentication

- Header: `Authorization: Bearer <api_key>` (the `vta_...` key from `agents.api_key`).
- The request is rejected with `401` if the header is missing or the key is unknown.
- On success, the resolved `agent_key` is used to scope all reads and writes.
- `agents.api_key_last_used_at` is updated on a successful authenticated request.

This auth layer is net-new (the app currently has only operator-facing HTML routes). It should be built as a reusable extractor so the future Hyperliquid execution gateway can use it too.

### Endpoints

#### `POST /api/v1/memories`

Create one memory.

Request body:

```json
{
  "symbol": "BTC",
  "timeframe": "1h",
  "memory_type": "plan",
  "summary": "Buy pullback above reclaimed support",
  "content": "## Thesis\nBTC reclaimed the prior breakout level...",
  "metadata": { "confidence": 0.72, "target": 67800 }
}
```

- `symbol`, `memory_type`, `summary`, `content` are required.
- `timeframe` is optional (omit or `null` for a general memory).
- `metadata` is optional (defaults to `{}`).
- `created_at` and `id` are server-generated.

Returns `201` with the created record (including `id` and `created_at`).

#### `GET /api/v1/memories`

List/roll-up the caller's own memories.

Query parameters (all optional, combinable; each narrows the result):

- `symbol` — exact match
- `timeframe` — exact match; **if omitted, returns the general (NULL-timeframe) memories**
- `memory_type` — exact match
- `since` — only memories with `created_at >= since` (RFC 3339 timestamp)
- `until` — only memories with `created_at < until` (RFC 3339 timestamp)
- `limit` — max rows (default and cap to be chosen during implementation, e.g. default 50, cap 200)

Results are ordered by `created_at DESC`.

Retrieval semantics:

- `?symbol=BTC&timeframe=1h&limit=24` — the daily loop reading the last 24 hourly memories.
- `?symbol=BTC&timeframe=15m&limit=4` — "last 4 fifteens".
- `?symbol=BTC&timeframe=1h&since=2026-06-15T00:00:00Z` — a time-window roll-up of hourly memories.
- `?symbol=BTC` (no `timeframe`) — the general/standing memories for BTC (`timeframe IS NULL`).

Note on omitting `timeframe`: because every cascading roll-up reads exactly one named timeframe, omitting `timeframe` is free to mean "the general, non-timeframed memories." There is intentionally **no** "all timeframes mixed" query in V1. A caller that genuinely needs multiple timeframes makes one call per timeframe. This is a deliberate tradeoff.

#### `GET /api/v1/memories/latest`

Fetch the latest currently valid memory per non-null timeframe for execution contexts such as the Hermes trading loop.

Required query parameters:

- `symbol` — exact match
- `memory_type` — exact match

Results:

- are scoped to the authenticated agent's `agent_key`
- only consider rows with a non-null `timeframe`
- return at most one row per timeframe
- return the newest non-stale row for each timeframe
- are ordered newest-first
- return an empty array if no current rows are valid

Response shape matches `GET /api/v1/memories`, with one extra field:

- `expires_at` — RFC 3339 timestamp when the memory becomes stale, or `null` if the row has no expiration

Stale analysis rules:

- if `metadata.stale_after` is present and parses as RFC 3339, it is used directly
- else if `metadata.valid_for_seconds` is a positive number, `expires_at = created_at + valid_for_seconds`
- else `memory_type="analysis"` falls back to timeframe defaults: `15m => 20m`, `1h => 90m`, `1d => 36h`, unknown analysis timeframe => `20m`
- non-analysis rows with no explicit staleness metadata currently have no implicit expiration

This endpoint exists alongside `GET /api/v1/memories`; it does not change the existing omitted-`timeframe` behavior there.

#### `GET /api/v1/memories/{id}`

Fetch a single memory by `id`, scoped to the caller's `agent_key`.

Returns `404` if the memory does not exist **or** belongs to a different agent (do not leak existence across agents).

### Error format

All errors use a simple JSON shape with an appropriate HTTP status:

```json
{ "error": "message describing what went wrong" }
```

- `401` — missing/invalid API key
- `404` — memory not found (or not owned by caller)
- `422` — invalid request body / missing required field
- `500` — unexpected server error

The agent API returns JSON errors, not the operator HTML error pages used by the existing web routes.

## Code Layout

Mirror the existing `src/agents/` module structure:

- `src/memory/mod.rs` — module exports.
- `src/memory/model.rs` — `MemoryRecord` row type, `CreateMemory` input, list-filter struct, and serializable API response types.
- `src/memory/store.rs` — DB access: `insert_memory`, `list_memories(filters)`, `get_memory(agent_key, id)`. Uses `sqlx` runtime queries (matching the rest of the codebase).

Auth + routes:

- An API-key auth extractor (reusable). Resolves Bearer token to an `agent_key` via `agents` and bumps `api_key_last_used_at`.
- JSON routes added under `/api/v1` in `src/web/routes.rs` (or a dedicated submodule), returning JSON responses and JSON errors.

Tests follow the existing integration test pattern used in `src/agents/store.rs`, `src/hyperliquid/queries.rs`, and `src/web/routes.rs`. DB-touching tests call `crate::test_db::pool()` to get a `sqlx::PgPool` backed by an embedded `pglite-oxide` PostgreSQL 17.5 server (in-process, real Postgres). No external service, no env var, and no `DATABASE_URL` is required for tests. The dev server (`cargo run`) keeps using the container Postgres at `localhost:15432/vibetrading` and is never touched by tests. The seven SSE body-read tests in `src/web/routes.rs` are `#[ignore]`d because the body reader hangs in pglite-oxide (no orchestrator-driven broadcast activity to drive the long-lived SSE stream); they were never actually executing before the test-DB refactor, since the old `DATABASE_URL` gate returned `None` and silently skipped them.

## Explicitly Deferred

The following ideas were part of the earlier, more ambitious memory design. They are intentionally **out of scope for V1** and preserved here as future direction:

- **Lineage graph** (`memory.record_links`) — directed relationships (`derived_from`, `supersedes`, `evaluates`, etc.) between records. For V1, references between memories live informally inside `metadata`.
- **Active-state projection** (`memory.active_state`) — a small table/view of the current memory per `(agent_key, symbol, timeframe)` for fast executor reads. For V1, "latest" is just `ORDER BY created_at DESC LIMIT 1`.
- **Job-run provenance** (`memory.job_runs`) — recording which job/model/prompt produced each memory. For V1, any such info can live in `metadata`.
- **Evaluations table** (`memory.evaluations`) and writing `evaluation_score` / `outcome_status` back onto records. For V1, evaluation is just another memory.
- **Cross-agent reads** — letting one agent read another agent's memories (e.g. a shared per-pair memory pool). V1 is strictly single-agent.
- **Separate `agent_key` per timeframe-role** (executor/analyst/evaluator). V1 uses one `agent_key` across all timeframes.
- **Validity windows / expiry** (`valid_until`) — explicit staleness hints. For V1, relevance is inferred from `created_at` + `timeframe`.
- **Semantic / embedding search** (`pgvector`). Not part of this system; retrieval is structured and time-based only.
- **Nullable `symbol`** for agent-wide / macro memories not tied to any instrument.
- **General cross-timeframe history fetches** — beyond the limited latest-per-timeframe view exposed by `GET /api/v1/memories/latest`.
- **Execution-intent linking** — connecting memories to submitted orders. That belongs to the Hyperliquid execution gateway subsystem and references `agent_key` separately.

These can be layered on later without breaking the V1 model, because the core table is append-only and `metadata` absorbs structured growth until a field justifies its own column.

## Summary

V1 of the memory system is a single append-only table, `memory.records`, owned per `agent_key`, scoped by `symbol` and optional `timeframe`, carrying a `summary`, a markdown `content` body, and optional `metadata`.

Agents save and retrieve memories through a small JSON API authenticated by API key (no `agent_id` in the path). Retrieval is time-ordered, supports the cascading roll-up pattern where each timeframe reads the one below it, and now includes a latest-valid-per-timeframe endpoint for execution loops.

Everything more advanced — lineage, active state, evaluations, expiry, search — is deliberately deferred until the simple version proves the feedback loop.
