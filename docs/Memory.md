# Memory System

Related docs:

- `README.md`
- `Agents.md`
- `OpenCode.md`
- `Architecture.md`

## Goal

The memory system stores time-ordered agent analysis so later runs can retrieve recent context and make decisions with continuity.

## Core Model

Each memory record is:

- owned by one agent via `agent_key`
- associated with one symbol
- optionally associated with one timeframe
- tagged with a free-text `memory_type`
- stored with `summary`, markdown `content`, and optional JSON `metadata`

The system is append-only. Memories are never updated in place.

## Storage

The memory subsystem lives in the `memory` schema and uses `memory.records` as the primary table plus `memory.links` for same-agent directed links between memories.

Important fields:

- `id`
- `created_at`
- `agent_key`
- `symbol`
- `timeframe`
- `memory_type`
- `summary`
- `content`
- `metadata`

Important `memory.links` fields:

- `source_memory_id`
- `target_memory_id`
- `link_type`
- `metadata`

## Retrieval

Agents read memories through the Vibetrading API and MCP tools.

Common retrieval patterns:

- recent memories for a symbol and timeframe
- latest valid memories per timeframe
- fresh `market_analysis` memories for execution handoff
- related source and review memories via `memory.links`

## Memory Contracts

The backend keeps `metadata` flexible JSON, but some OpenCode flows rely on stable keys.

Current important contract:

- `memory_type = "market_analysis"` is the primary execution handoff for trading
- trading should not open new exposure when no fresh `market_analysis` memory exists
- `memory_type = "agent_learnings"` is the append-only durable learning stream for one agent
- `memory_type = "daily_review"` records one review for an agent-scoped UTC window
- market-analysis memories should link back to source analyses with `derived_from`
- daily-review memories should link to reviewed memories with `reviews` and to new learnings with `updates_learnings`
