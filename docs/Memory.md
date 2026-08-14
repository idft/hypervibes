# Memory System

Related docs:

- `README.md`
- `Agents.md`
- `OpenCode.md`
- `Architecture.md`

## Goal

The memory system stores time-ordered, agent-owned analysis and review context
so later runs can make decisions with continuity.

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

Agents read memories through the authenticated HyperVibes API and workspace
MCP tools. All reads and writes are scoped to the calling `agent_key`.

Common retrieval patterns:

- recent memories for a symbol and timeframe
- latest valid memories per timeframe
- fresh `market_analysis` memories for execution handoff
- related source and review memories via `memory.links`

## Memory Contracts

The backend keeps `metadata` flexible JSON, but some OpenCode flows rely on stable keys.

Current important contract:

- `memory_type = "market_analysis"` is the primary execution handoff for trading
- market-analysis memories must omit `timeframe` so the stored value is `NULL`
- OpenCode trading instructions require a fresh `market_analysis` memory before
  opening new exposure; this is a workflow contract, not an API-level order
  validation rule today
- market-analysis metadata declares `execution_state` as `execute`,
  `conditional`, `wait`, `manage_existing`, or `cancel_entries`
- `conditional` memories declare a finite `confirmation_timeframes` list and
  machine-readable `confirmation_rules`; trading may fetch only those closed
  candles and run the canonical analyzer solely to verify those rules
- trading treats the selected memory's thesis, direction, levels, confidence,
  and risk parameters as immutable; it must stand down when confirmation data,
  analyzer output, or a declared rule is unavailable or fails
- `memory_type = "agent_learnings"` is the append-only durable learning stream for one agent
- `memory_type = "daily_review"` records one review for an agent-scoped UTC window
- market-analysis memories should link back to source analyses with `derived_from`
- daily-review memories should link to reviewed memories with `reviews` and to new learnings with `updates_learnings`

Coding result memories use:

- `memory_type = "analysis_coding"`
- `symbol = "__agent__"`
- no timeframe
- metadata containing task/run IDs, mode, outcome, manifest hashes, changed paths, and validation results

The application writes these memories only after no-change completion or
successful promotion. Coding results link to the requesting review with
`responds_to` and to validated same-agent evidence with `derived_from`.
