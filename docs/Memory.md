# Memory / Analysis System

WARNING: THIS IS A WORK IN PROGRESS AND IS IDEAS ONLY.
NOTHING IN THIS FILE SHOULD BE IMPLEMENTED YET.

Related docs:

- `README.md` for top-level V2 direction
- `Agents.md` for the agent registry and the mapping from `agent_key` to execution account ownership
- `Hyperliquid.md` for the account-history and reconciliation subsystem this memory system reads from
- `Architecture.md` for implementation-language and runtime options

## Goal

Build a layered memory and analysis system for trading agents that combines:

- short-term tactical analysis
- medium-term market context
- long-term evaluation and learning
- low-cost execution against higher-quality analysis

The core idea is that different jobs run on different candle boundaries, each with a specific responsibility, and each writing structured memory back into the system.

The system should not primarily store freeform text notes.

It should store structured market beliefs, trade plans, outcomes, and reflections, with optional freeform commentary attached.

## Scope Boundary

This document is only about the memory subsystem.

The memory subsystem should own:

- memory records
- memory links and lineage
- active memory state
- evaluations
- reflections

The memory subsystem should not own raw account or exchange history.

That should be handled by a separate subsystem later, likely in a different Postgres schema, and the memory system can reference that external data when needed.

The memory subsystem also should not be the canonical owner of exchange order submission records.

That should live in the Hyperliquid subsystem, with the agents module defining which execution account belongs to which `agent_key`.

## Execution Intent Boundary

The memory subsystem needs a clean way to link analysis to actual exchange behavior.

That does not mean memory should own orders or fills directly.

Instead, the system should separate three layers:

1. memory analysis and trade rationale
2. execution intent and submitted order records
3. exchange-confirmed history and reconciliation

The current direction is:

- memory owns the analysis objects and rationale
- Hyperliquid owns execution-intent and order-submission records
- Hyperliquid also owns fills, funding, fees, and cash-event reconciliation

This preserves the ability to evaluate:

- whether the thesis was good
- whether the intended execution made sense
- whether the actual exchange result matched the intent

### Why This Separation Matters

Without an execution-intent layer, the system can see analysis memories and later fills, but it cannot reliably answer:

- did the agent intend to trade here?
- which memory records justified the order?
- was the order submitted as intended?
- was there no fill because the idea was bad, or because execution never happened?

That means the V2 design should explicitly link memory records to canonical execution-intent records stored outside the memory schema.

### Preferred Order Submission Pattern

The preferred direction is for agents not to send orders straight to Hyperliquid without local tracking.

A better pattern is an internal Hyperliquid execution gateway that:

- accepts an order request from the agent
- records execution-intent metadata first
- stores links to `agent_key` and related memory record IDs
- forwards the order to Hyperliquid
- later reconciles the submitted order against exchange events

This gateway does not need to be a separate deployable.

It can live inside the same main application binary as the Hyperliquid subsystem.

The gateway should be thought of as an audited order router, not as a transparent clone of the full Hyperliquid API.

### Fallback Pattern

If an agent is ever allowed to send orders directly to Hyperliquid, then that agent must also write the matching execution-intent record.

That is less desirable because it makes the audit chain easier to break.

So the current design should lean toward a gateway model.

## Core Idea

V2 should treat memory as a stack of linked artifacts:

- observations
- hypotheses
- plans
- outcomes
- reflections

These artifacts should build on each other across timeframes.

Higher timeframes should provide context and constraints.
Lower timeframes should provide timing and execution.

Example:

- daily job defines regime and broad risk conditions
- hourly job defines active market narrative and key levels
- 15m job defines tactical opportunities
- 1m job executes inside those boundaries

## Why Structured Memory

Plain text memory is easy to generate, but it causes problems:

- hard to retrieve the right information
- hard to compare predictions against outcomes
- hard to expire stale ideas
- hard to measure whether an agent is improving
- easy for the system to become noisy and repetitive

Structured memory allows the system to:

- rank and filter current beliefs
- link decisions to prior analysis
- measure actual accuracy vs expected outcomes
- learn which types of analysis work in which regimes

## Main Loops

### 1m Execution Loop

Purpose:

- place, adjust, reduce, or cancel orders
- manage active positions
- react cheaply and quickly to recent market changes

Model:

- low-cost, fast model

Reads:

- latest approved 15m tactical state
- latest approved 1h context
- current risk state from an external trading subsystem
- open positions from an external trading subsystem
- relevant execution constraints

Writes:

- execution decision records
- references to submitted order actions
- reason codes
- memory IDs used for justification
- execution quality notes

This loop should not do broad market reasoning.
It should execute within the boundaries created by higher-timeframe analysis.

Canonical order-submission records should live in the Hyperliquid subsystem, not in the memory schema.

### 15m Tactical Analysis Loop

Purpose:

- produce short-term market interpretation
- identify the best current tactical setup
- define entry conditions and invalidation
- guide the 1m executor

Model:

- smarter, more expensive model

Reads:

- current market snapshot
- latest 1h context
- latest daily state
- recent unresolved 15m hypotheses
- recent evaluation snippets if useful

Writes:

- tactical bias
- best setup candidates
- confidence
- expected horizon
- expected move
- conditions for entry
- invalidation
- relation to higher-timeframe context

This loop should focus on the next few candles or next 1-2 hours, not all-day strategy.

### 1h Context / Regime Loop

Purpose:

- define medium-term market context
- interpret current session structure
- decide whether short-term setups should be trusted aggressively or faded

Model:

- medium cost model

Reads:

- current market snapshot
- latest daily state
- recent 15m summaries
- recent outcome metrics

Writes:

- market regime
- directional bias
- key levels
- session narrative
- volatility / momentum context
- setup quality guidance for lower timeframes

This loop should answer questions like:

- are we trending or mean-reverting?
- are breakouts working today?
- should 15m setups be traded aggressively or conservatively?
- what invalidates the current market narrative?

### 1d Evaluation / Learning Loop

Purpose:

- compare expectations against actual outcomes
- score prior analysis quality
- detect repeated failure modes
- update longer-lived beliefs

Model:

- medium or expensive model depending on batch size

Reads:

- prior day hypotheses
- plans
- execution logs
- open/closed trade results
- performance grouped by setup, regime, confidence, and session

Writes:

- outcome summaries
- evaluation scores
- reflections
- updated regime beliefs
- repeated error patterns
- calibration insights
- prompt / policy adjustment ideas

This loop should not place trades.
Its job is learning and memory quality improvement.

## Separation of Responsibilities

The system works best when each layer has a narrow job.

Daily:

- world model
- broad market regime
- strategic preferences
- major risks

Hourly:

- active context
- intraday narrative
- key levels
- current market conditions

15m:

- tactical opportunities
- concrete setups
- triggers and invalidations

1m:

- execution
- order timing
- position management

The lower loops should inherit context from the higher loops instead of recomputing everything from scratch.

## Memory Types

### Observation

Facts about the market at a point in time.

Examples:

- trend state
- volatility regime
- funding rate condition
- liquidation cluster nearby
- support / resistance structure

### Hypothesis

A belief about what will likely happen.

Examples:

- BTC likely reclaims VWAP and pushes to session high in the next 90 minutes
- ETH likely fails breakout attempts while funding remains crowded long

### Plan

A tradeable expression of the hypothesis.

Examples:

- long only if pullback holds above level X with improving delta
- short only if rejection occurs at level Y and momentum confirms

### Outcome

What actually happened.

Examples:

- target reached
- invalidation hit
- chop / no follow-through
- thesis never triggered
- thesis correct but execution poor

### Reflection

Why the hypothesis or plan succeeded or failed.

Examples:

- breakout calls worked only when aligned with 1h trend and rising OI
- mean reversion calls underperformed during NY session trend expansion
- high-confidence scores were too generous in low-liquidity conditions

## Recommended Memory Record Shape

Each memory record should be structured enough for retrieval and evaluation.

Suggested fields:

- `id`
- `timestamp`
- `symbol`
- `timeframe`
- `memory_type`
- `source_job`
- `valid_from`
- `valid_until`
- `market_regime`
- `summary`
- `thesis`
- `primary_scenario`
- `counter_scenario`
- `conditions`
- `invalidation`
- `confidence`
- `expected_horizon`
- `expected_move`
- `recommended_action`
- `risk_notes`
- `parent_memory_ids`
- `decision_ids`
- `outcome_status`
- `evaluation_score`
- `tags`

Optional fields:

- `freeform_reasoning`
- `features_snapshot`
- `raw_market_context`
- `llm_metadata`

The important point is that later jobs can compare expectation vs reality in a machine-usable way.

## Chosen Backend

The chosen backend for the V2 memory system is Postgres.

This memory system should use its own dedicated schema rather than `public`.

Recommended schema name:

- `memory`

Rationale:

- memory records are structured, relational, and time-based
- multiple jobs will read and write concurrently
- the system needs strong lineage, filtering, and auditability
- active-state projections are easier to build in Postgres
- memory should stay isolated from unrelated application tables

This document is not proposing a generic AI memory framework as the source of truth.

The source of truth should be Postgres tables inside the `memory` schema.

Implementation guidance:

- use typed columns for critical fields like `symbol`, `timeframe`, `memory_type`, `valid_from`, `valid_until`, and `confidence`
- use `JSONB` for flexible structured payloads that may evolve over time
- keep both active-state tables or views and historical archive tables in the same `memory` schema
- keep account history, fills, positions, and exchange sync state in a separate schema managed by another subsystem
- allow memory records to store external references to those other schemas when evaluation requires them

At least for the first version, Postgres alone should be enough.

Do not depend on a separate AI memory product for canonical storage, retrieval, or active trading state.

## Proposed Postgres Schema

This section proposes the initial database design for the memory subsystem inside the `memory` Postgres schema.

This is intentionally scoped to memory only.

It does not include:

- account history tables
- fills or raw exchange events
- position snapshots
- balance history
- sync cursors for exchange ingestion

Those belong to a separate subsystem and schema.

### Namespace

- schema name: `memory`

All memory tables should live under this schema.

Examples:

- `memory.records`
- `memory.record_links`
- `memory.job_runs`
- `memory.active_state`
- `memory.evaluations`

### Design Approach

Use one canonical table for memory objects, then add a few small supporting tables around it.

This keeps the system simple while still allowing:

- structured retrieval
- lineage between memories
- active-state projection
- later evaluation against actual results
- auditability of which job created which memory

### Multi-Agent / Pair Scoping

The memory system needs to support multiple AI agents.

Even if the initial design is one agent per trading pair, `symbol` alone is not a sufficient ownership key.

Why:

- there may eventually be more than one agent on the same pair
- one pair may have separate sandbox, paper, and live agents
- one pair may have multiple strategy personalities or profiles later
- active state should be isolated per agent, not shared accidentally by symbol alone

Because of that, the schema should use an explicit agent scope.

Recommended fields:

- `agent_key TEXT NOT NULL`
- `agent_role TEXT`

Suggested meaning:

- `agent_key` is the stable identity of one trading agent memory stream
- `agent_role` is optional and can describe the responsibility, such as `executor`, `analyst`, or `evaluator`

Examples of `agent_key`:

- `btc-usd-live`
- `eth-usd-live`
- `btc-usd-sandbox`

In the simplest setup, one `agent_key` maps to one trading pair.

That means the effective memory scope becomes:

- `(agent_key, symbol, timeframe)`

In practice, most queries should lead with `agent_key` first, then `symbol`, then timeframe or memory type.

This gives you clean isolation now while still leaving room for multiple agents per pair later.

### Agent Registry Ownership

The canonical registry of known agents should not live in the memory schema.

That should live in the dedicated agents subsystem instead.

Current direction:

- `agents.registry` is the canonical owner of `agent_key`
- memory tables reference `agent_key` but do not define agent identity themselves
- the agents subsystem also owns the mapping from `agent_key` to execution-account ownership

### 1. `memory.records`

This is the core table.

It stores the canonical memory objects written by analysis and reflection jobs.

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `updated_at TIMESTAMPTZ NOT NULL`
- `agent_key TEXT NOT NULL`
- `agent_role TEXT`
- `symbol TEXT NOT NULL`
- `timeframe TEXT NOT NULL`
- `memory_type TEXT NOT NULL`
- `source_job TEXT NOT NULL`
- `status TEXT NOT NULL`
- `valid_from TIMESTAMPTZ`
- `valid_until TIMESTAMPTZ`
- `market_regime TEXT`
- `summary TEXT NOT NULL`
- `thesis TEXT`
- `primary_scenario TEXT`
- `counter_scenario TEXT`
- `confidence NUMERIC(5,4)`
- `expected_horizon TEXT`
- `outcome_status TEXT`
- `evaluation_score NUMERIC(5,4)`
- `run_id UUID`
- `tags TEXT[] NOT NULL DEFAULT '{}'`
- `conditions JSONB NOT NULL DEFAULT '{}'::jsonb`
- `invalidation JSONB NOT NULL DEFAULT '{}'::jsonb`
- `expected_move JSONB NOT NULL DEFAULT '{}'::jsonb`
- `recommended_action JSONB NOT NULL DEFAULT '{}'::jsonb`
- `risk_notes JSONB NOT NULL DEFAULT '{}'::jsonb`
- `payload JSONB NOT NULL DEFAULT '{}'::jsonb`

Notes:

- `agent_key` is the primary ownership boundary for memory
- `memory_type` should initially map to the concepts in this doc: `observation`, `hypothesis`, `plan`, `outcome`, `reflection`
- `status` can track lifecycle such as `active`, `expired`, `superseded`, `evaluated`, or `archived`
- `payload` is the escape hatch for additional structured fields without constant migrations
- if a memory record later needs a stable relationship to another subsystem, prefer an explicit column or link table with a real foreign key

This table is the source of truth for memory content.

### 2. `memory.record_links`

This table stores directed relationships between memory records.

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `agent_key TEXT NOT NULL`
- `from_record_id UUID NOT NULL`
- `to_record_id UUID NOT NULL`
- `link_type TEXT NOT NULL`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested `link_type` values:

- `derived_from`
- `refines`
- `uses_context_from`
- `evaluates`
- `invalidates`
- `supersedes`
- `supports`

Examples:

- a `15m` plan can `uses_context_from` the latest `1h` context memory
- a daily reflection can `evaluates` a set of prior hypotheses
- a newer hourly context can `supersedes` the older one

This is the lineage graph for the memory system.

### 3. `memory.job_runs`

This table records the provenance of each memory-writing job.

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `job_name TEXT NOT NULL`
- `agent_key TEXT NOT NULL`
- `agent_role TEXT`
- `symbol TEXT`
- `timeframe TEXT`
- `triggered_at TIMESTAMPTZ NOT NULL`
- `started_at TIMESTAMPTZ`
- `completed_at TIMESTAMPTZ`
- `status TEXT NOT NULL`
- `model_name TEXT`
- `prompt_version TEXT`
- `input_context JSONB NOT NULL DEFAULT '{}'::jsonb`
- `output_summary TEXT`
- `error_text TEXT`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Purpose:

- know which job created which memory
- support audits and debugging
- preserve prompt/model provenance without storing everything in freeform logs

`memory.records.run_id` should reference this table.

### 4. `memory.active_state`

This table projects the currently active memory state that the live system should consume.

It should stay small.

Suggested columns:

- `state_key TEXT PRIMARY KEY`
- `agent_key TEXT NOT NULL`
- `agent_role TEXT`
- `symbol TEXT NOT NULL`
- `timeframe TEXT NOT NULL`
- `record_id UUID NOT NULL`
- `as_of TIMESTAMPTZ NOT NULL`
- `expires_at TIMESTAMPTZ`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested `state_key` examples:

- `btc-usd-live:btc:1d:regime`
- `btc-usd-live:btc:1h:context`
- `btc-usd-live:btc:15m:tactical_plan`
- `btc-usd-live:btc:1d:reflection`

Notes:

- this can start as a physical table updated by writers
- later it could become a view or materialized view if that proves cleaner
- the 1m executor should read mostly from this table, plus external trading/account state

### 5. `memory.evaluations`

This table stores structured scoring of prior memory records.

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `agent_key TEXT NOT NULL`
- `target_record_id UUID NOT NULL`
- `run_id UUID`
- `window_start TIMESTAMPTZ`
- `window_end TIMESTAMPTZ`
- `directional_correct BOOLEAN`
- `expected_move_hit BOOLEAN`
- `invalidation_hit BOOLEAN`
- `followed_in_execution BOOLEAN`
- `score NUMERIC(5,4)`
- `summary TEXT NOT NULL`
- `details JSONB NOT NULL DEFAULT '{}'::jsonb`

Purpose:

- separate raw memory content from later scoring
- support repeated re-evaluation if needed
- make it easy to analyze which memory patterns are working

This table should usually point back to a target row in `memory.records`.

### Optional Later Tables

These are not required for the first version, but may be useful later:

- `memory.tags` if tags need normalization
- `memory.saved_searches` for operator tooling
- `memory.embeddings` if semantic search is added later via `pgvector`

For V1 of the memory backend, the five core tables above should be enough.

### Suggested Constraints

At minimum:

- `memory.records.memory_type` should be constrained to known values
- `memory.records.status` should be constrained to known lifecycle values
- `memory.records.valid_until` should be greater than or equal to `valid_from` when both are set
- `memory.record_links.from_record_id` and `to_record_id` should not be equal
- `memory.evaluations.score` and `memory.records.evaluation_score` should use a consistent range
- if the agents subsystem is present, all `agent_key` columns should reference `agents.registry`

### Suggested Indexes

Initial indexes should focus on the common retrieval paths.

For `memory.records`:

- `(agent_key, symbol, timeframe, memory_type, created_at DESC)`
- `(agent_key, symbol, timeframe, status, valid_until)`
- `(run_id)`
- `GIN (tags)`
- `GIN (payload)` only if query patterns justify it

For `memory.record_links`:

- `(agent_key)`
- `(from_record_id)`
- `(to_record_id)`
- `(link_type)`

For `memory.job_runs`:

- `(job_name, triggered_at DESC)`
- `(agent_key, symbol, timeframe, triggered_at DESC)`

For `memory.active_state`:

- `(agent_key, symbol, timeframe)`
- `(record_id)`

For `memory.evaluations`:

- `(agent_key, target_record_id, created_at DESC)`
- `(target_record_id, created_at DESC)`
- `(run_id)`

### Recommended Initial Rules

To keep the first version clean:

- only `memory.records` should hold canonical memory content
- `memory.active_state` should point at records, not duplicate large text blobs
- reflections should still be stored in `memory.records`
- evaluations should live in `memory.evaluations` and optionally write summary values back onto `memory.records`
- use cross-schema foreign keys where ownership is clear and integrity matters
- if a cross-subsystem relationship matters enough to store, model it explicitly
- all live reads and writes should be scoped by `agent_key`

### Example Lifecycle

1. A `15m` job writes a new row to `memory.job_runs`.
2. That job writes one or more rows to `memory.records`.
3. It links those rows to higher-timeframe context using `memory.record_links`.
4. It updates `memory.active_state` so the latest tactical plan becomes the current live input.
5. Later, the daily evaluator writes a row to `memory.evaluations` for the original record.
6. The evaluator may also write a new `reflection` record into `memory.records` and link it back with `evaluates`.

### Example Agent-Scoped Queries

The examples below assume one live agent:

- `agent_key = 'btc-usd-live'`
- `symbol = 'BTC-USD'`

The important point is that every live query starts with `agent_key`.

#### Example: 1m Executor Reads Active Memory

The executor should read only the current active state for its own agent.

Pseudo-query:

```sql
SELECT s.state_key, r.*
FROM memory.active_state s
JOIN memory.records r ON r.id = s.record_id
WHERE s.agent_key = 'btc-usd-live'
  AND s.symbol = 'BTC-USD'
  AND s.expires_at IS NULL OR s.expires_at > now();
```

Expected result set:

- latest daily regime memory
- latest hourly context memory
- latest 15m tactical plan
- latest reflection or calibration memory if one is active

The executor can combine that with position and risk data from the external trading subsystem.

#### Example: 15m Analyst Writes New Tactical Plan

The 15m job creates a job run, inserts a new plan record, links it to higher-timeframe context, then updates active state.

Pseudo-flow:

```sql
INSERT INTO memory.job_runs (id, created_at, job_name, agent_key, symbol, timeframe, triggered_at, status)
VALUES (..., now(), 'analysis_15m', 'btc-usd-live', 'BTC-USD', '15m', now(), 'completed');

INSERT INTO memory.records (
  id,
  created_at,
  updated_at,
  agent_key,
  symbol,
  timeframe,
  memory_type,
  source_job,
  status,
  valid_from,
  valid_until,
  summary,
  thesis,
  confidence,
  run_id,
  conditions,
  invalidation,
  expected_move,
  recommended_action
)
VALUES (
  ..., now(), now(), 'btc-usd-live', 'BTC-USD', '15m', 'plan', 'analysis_15m', 'active',
  now(), now() + interval '2 hours',
  'Buy pullback above reclaimed support',
  'BTC likely continues higher if pullback holds above key level',
  0.72,
  ...,
  '{"entry":"hold above 67100"}'::jsonb,
  '{"level":66850}'::jsonb,
  '{"target":67800}'::jsonb,
  '{"side":"buy"}'::jsonb
);
```

Then link it to the current hourly and daily context records for the same `agent_key`.

#### Example: 1h Analyst Reads Recent 15m History

The hourly job should read only recent records for its own agent.

Pseudo-query:

```sql
SELECT *
FROM memory.records
WHERE agent_key = 'btc-usd-live'
  AND symbol = 'BTC-USD'
  AND timeframe = '15m'
  AND memory_type IN ('hypothesis', 'plan', 'outcome')
  AND created_at >= now() - interval '8 hours'
ORDER BY created_at DESC;
```

This avoids mixing in memory from:

- other symbols
- sandbox agents
- future alternate BTC agents

#### Example: Daily Evaluator Scores Prior Plans

The daily evaluator should score prior records for a single agent scope.

Pseudo-query:

```sql
SELECT r.*
FROM memory.records r
WHERE r.agent_key = 'btc-usd-live'
  AND r.symbol = 'BTC-USD'
  AND r.timeframe = '15m'
  AND r.memory_type = 'plan'
  AND r.created_at >= now() - interval '1 day';
```

For each target plan, it can then write an evaluation row:

```sql
INSERT INTO memory.evaluations (
  id,
  created_at,
  agent_key,
  target_record_id,
  score,
  directional_correct,
  expected_move_hit,
  summary,
  details
)
VALUES (
  ..., now(), 'btc-usd-live', ..., 0.81, true, true,
  'Plan was directionally correct and target was reached inside the expected window',
  '{"window":"2h","target_hit":true}'::jsonb
);
```

#### Example: Same Pair, Different Agent

If another agent also trades BTC, its memory remains isolated by `agent_key`.

Example alternate agent:

- `agent_key = 'btc-usd-sandbox'`

Even if both agents use:

- `symbol = 'BTC-USD'`
- `timeframe = '15m'`

their active state and historical memory stay separate because all reads and writes are scoped by `agent_key`.

## Active State vs Archive

The system should have two different storage concepts.

### Active State

Small, current, distilled state used directly by the executor.

This should include:

- latest daily state
- latest hourly context
- latest active 15m tactical plan
- latest active reflection or calibration state if needed

The 1m executor should mostly read from active state.

If the executor also needs position, risk, or account state, that should come from a separate trading/account subsystem rather than from the memory schema itself.

### Archive / History

Full historical record of:

- past analyses
- prior plans
- outcomes
- reflections
- execution decisions

This exists for audit, retrieval, and evaluation.

The executor should not scan large historical text logs every minute.

## How Memories Build on Each Other

The memory graph should be linked, not just accumulated.

Flow:

1. Daily job creates strategic state.
2. Hourly job reads daily state and refines it into current intraday context.
3. 15m job reads hourly and daily state and produces tactical setups.
4. 1m job reads active 15m and 1h context and makes execution decisions.
5. Daily evaluator later links analysis to results and writes reflections.

This creates inheritance across timeframes:

- high timeframe gives context
- low timeframe gives precision
- evaluation loop improves future context and precision

## Expectation vs Reality

A key feature of V2 should be explicit evaluation.

Each hypothesis or plan should later be graded against what actually happened.

Suggested evaluation questions:

- was the directional call correct?
- did the expected move happen?
- did it happen within the expected time window?
- did invalidation occur?
- did the trade make money after fees and slippage?
- was the thesis useful even if no trade was taken?
- did execution follow the plan?
- in what market regime did this succeed or fail?

This allows the daily evaluator to produce meaningful learning, not just PnL summaries.

## Reflection and Improvement

The real value of memory comes from reflection.

The system should learn things like:

- which setup families are currently working
- which confidence levels are well-calibrated
- which regimes break certain tactics
- which session conditions lead to poor follow-through
- when 15m analysis should be trusted more or less
- when execution quality is the main issue rather than analysis quality

Examples of useful reflections:

- breakout theses had strong directional accuracy but weak net PnL due to late entries
- high-confidence 15m calls only performed well when 1h and daily bias aligned
- mean reversion setups underperformed during trend expansion sessions
- invalidation levels were too loose during high-volatility conditions

## Important Design Principles

### 1. Structured over freeform

Text is useful as explanation, but the system should run on structured objects.

### 2. Expiry matters

Every analysis item needs a validity window.
Old tactical analysis should expire quickly.

### 3. Narrow roles

Do not let every agent do every type of reasoning.

### 4. Active state should stay small

The executor should read distilled context, not a giant memory log.

### 5. Evaluate analysis separately from execution

A good thesis can lose money from bad execution.
A bad thesis can make money by luck.
These must be separated.

### 6. Store counter-scenarios

Each analysis should include both:

- primary expected path
- alternate invalidating path

This makes later evaluation far more useful.

## Retrieval Rules By Job

### 1m Executor Reads

- latest active 15m tactical memory
- latest active 1h context memory
- latest daily regime summary
- current account / risk / position state from an external trading subsystem

### 15m Analyst Reads

- latest 1h context
- latest daily regime state
- market snapshot
- recent unresolved 15m items
- selected recent evaluation notes

### 1h Analyst Reads

- recent 15m summaries
- latest daily state
- market snapshot
- recent outcome metrics

### 1d Evaluator Reads

- prior day analyses
- plans
- execution and outcome data from an external trading subsystem
- grouped performance metrics
- notable regime changes

## Example System Output Chain

Example:

1. Daily memory says market is in risk-on trend continuation regime.
2. Hourly memory says BTC is consolidating above prior breakout level with strong session structure.
3. 15m memory says buy pullback above level X targeting level Y within 2 hours.
4. 1m executor enters only if micro pullback confirms and risk limits allow.
5. Later evaluation checks whether:
   - level X held
   - target Y was reached
   - the move occurred in the expected time window
   - the actual trade execution matched the plan
   - similar setups are working or failing lately

## What To Avoid

- freeform text-only memory as the main system
- letting low timeframe agents reread huge historical logs
- no validity windows on analysis
- storing opinions without measurable expectations
- learning from PnL alone
- letting the 1m agent invent strategy instead of executing it

## Minimal Practical V2

A reasonable first version of this system could be:

1. structured memory records instead of plain notes
2. 15m tactical analysis writer
3. 1h context writer
4. 1d evaluation writer
5. 1m executor that reads only distilled active state
6. linking analysis memory to execution decisions and outcomes

This would be enough to create a real feedback loop without overbuilding the first version.

## Open Questions

Some design questions still need decisions:

1. Should active state be implemented as physical tables, normal views, or materialized views inside the `memory` schema?
2. Should each symbol have fully separate memory chains, or should there also be global market memory?
3. Should the daily evaluator update only memory, or also propose prompt / policy changes?
4. Should confidence be a free numeric score, a bucketed label, or both?
5. Should the 1m executor be allowed to override higher-timeframe plans, or only abstain from acting?
6. How much raw market data should be embedded into memory records versus referenced externally?

## Summary

The goal of the V2 memory system is not to create a larger collection of AI notes.

The goal is to create a layered trading memory that:

- captures market context across timeframes
- converts analysis into explicit tradeable plans
- links plans to execution and outcomes
- evaluates reality vs expectations
- produces reflections that improve future decisions

If designed correctly, this becomes a system where:

- daily shapes strategic bias
- hourly shapes intraday context
- 15m shapes tactical setups
- 1m executes efficiently
- daily evaluation helps the whole stack improve over time
