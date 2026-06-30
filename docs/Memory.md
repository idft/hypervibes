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

The memory subsystem lives in the `memory` schema and currently uses `memory.records` as the primary table.

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

## Retrieval

Agents read memories through the Vibetrading API and MCP tools.

Common retrieval patterns:

- recent memories for a symbol and timeframe
- latest valid memories per timeframe
- fresh `market_analysis` memories for execution handoff

## Memory Contracts

The backend keeps `metadata` flexible JSON, but some OpenCode flows rely on stable keys.

Current important contract:

- `memory_type = "market_analysis"` is the primary execution handoff for trading
- trading should not open new exposure when no fresh `market_analysis` memory exists

## Deprecated API Note

`/api/v1/job-context` remains available temporarily, but memory retrieval should not depend on that endpoint long term.
