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

Each loop reads **one specific timeframe** at a time. The system does not need a "give me all timeframes mixed together" query.

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
| `agent_key`   | `TEXT NOT NULL REFERENCES agents.registry(agent_key) ON DELETE CASCADE` | ownership boundary                         |
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
4. Foreign key `agent_key -> agents.registry(agent_key) ON DELETE CASCADE`.

The `.down.sql` drops the table and the schema.

## Agent API

The agent API is JSON and authenticated by API key. It does **not** include an `agent_id` in any path; the API key determines which agent the memories belong to.

All routes are under `/api/v1/memories`.

### Authentication

- Header: `Authorization: Bearer <api_key>` (the `vta_...` key from `agents.registry.api_key`).
- The request is rejected with `401` if the header is missing or the key is unknown.
- On success, the resolved `agent_key` is used to scope all reads and writes.
- `agents.registry.api_key_last_used_at` is updated on a successful authenticated request.

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

- An API-key auth extractor (reusable). Resolves Bearer token to an `agent_key` via `agents.registry` and bumps `api_key_last_used_at`.
- JSON routes added under `/api/v1` in `src/web/routes.rs` (or a dedicated submodule), returning JSON responses and JSON errors.

Tests follow the existing `DATABASE_URL`-gated integration test pattern used in `src/agents/store.rs` and `src/web/routes.rs`.

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
- **Cross-timeframe single-call fetch** — one request returning multiple timeframes at once.
- **Execution-intent linking** — connecting memories to submitted orders. That belongs to the Hyperliquid execution gateway subsystem and references `agent_key` separately.

These can be layered on later without breaking the V1 model, because the core table is append-only and `metadata` absorbs structured growth until a field justifies its own column.

## Summary

V1 of the memory system is a single append-only table, `memory.records`, owned per `agent_key`, scoped by `symbol` and optional `timeframe`, carrying a `summary`, a markdown `content` body, and optional `metadata`.

Agents save and retrieve memories through a small JSON API authenticated by API key (no `agent_id` in the path). Retrieval is time-ordered and supports the cascading roll-up pattern where each timeframe reads the one below it.

Everything more advanced — lineage, active state, evaluations, expiry, search — is deliberately deferred until the simple version proves the feedback loop.
