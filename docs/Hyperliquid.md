# Hyperliquid Module

Related docs:

- `README.md` for top-level V2 direction
- `Agents.md` for the registry that maps `agent_key` to execution-account ownership
- `Memory.md` for the subsystem that will later consume realized account history for evaluation
- `Architecture.md` for implementation-language and runtime options

## Goal

Define the planned Hyperliquid integration module for V2.

This module should be responsible for:

- reconciling Hyperliquid account activity into local Postgres
- maintaining a durable local account activity journal
- supporting efficient queries for agent evaluation and UI use
- ingesting and repairing exchange history with exact exchange semantics

This module should be separate from the memory subsystem, but it can still live inside the same application binary.

## Current Direction

The current design direction is:

- one application binary
- one Postgres database
- separate Postgres schemas per subsystem
- a dedicated `hyperliquid` schema for this module
- one execution account per agent
- startup reconciliation before live operation begins
- app-owned execution gateway for future order submission
- `hypersdk` reused as the sole Hyperliquid library
- historical bootstrap plus periodic polling as the primary correctness path

This should be treated as a distinct subsystem alongside:

- the `memory` subsystem
- the web UI

## `hypersdk` Integration Direction

The current direction is to use the `hypersdk` Rust crate as the sole Hyperliquid SDK.

The Hyperliquid subsystem should:

- use `hypersdk` for instrument loading, live account state, the live WebSocket, and (later) signing/order submission
- keep execution and journaling logic owned by the app itself
- own a thin raw HTTP `/info` client for the account-history endpoints `hypersdk` does not expose (`userFillsByTime`, `userFunding`, `nonUserFundingLedgerUpdates`, `historicalOrders`)

This matters because the long-term execution model is app/agent driven.

### `hypersdk` surface used today

- Instruments: `hypercore::mainnet().perps()` (perp markets on the default DEX) is loaded at startup and mapped into `hyperliquid.instruments`.
- Live WebSocket: `hypercore::mainnet_ws()` driven by `hyperliquid::live_ws`, converted through `live_convert` into the app's domain types and journal rows.
- Signing/order submission: reserved for the future execution gateway.

### Resulting v1 decision

- `hypersdk` is used in the journal for **instrument sync only** today.
- All account history is fetched through an **app-owned raw HTTP `/info` client** (`raw_http`). The `/info` endpoints are public and require no signing.
- HTTP polling is the canonical correctness path; the live WebSocket is a fast-path for live fills/order updates.

## Scope

The Hyperliquid module should focus on reconciling account activity.

Initial scope:

- deposits
- withdrawals
- funding paid / received
- fees
- fills / trades
- transfers affecting the tracked account
- realized trade outcomes needed for evaluation

Explicitly not required for the first version:

- tracking open positions as a primary persisted concern
- storing rapidly changing live account state such as current position snapshots or current equity
- vault activity
- staking rewards
- staking delegations
- webhook delivery to AI agents

The module should support multiple accounts in schema design, even if one deployment may only use one account initially.

The current direction is one execution account per agent, with the agents module owning that mapping.

## Core Design Principle

This module should not be modeled primarily as a mirror of current account state.

It should be modeled as a durable local `account activity journal`.

That journal becomes the source of truth for:

- cash movements
- trading fills
- funding
- fees
- realized PnL-related activity

Derived summaries and UI views should be built on top of that journal.

## Why a Journal Model

A journal model makes it easier to:

- reconcile exactly against exchange history
- preserve immutable historical records
- repair or replay ingestion when needed
- support efficient evaluation queries for agents
- produce daily or per-symbol summaries without losing raw fidelity

This is a better fit than trying to store only snapshots or current balances.

The local database should focus on historical account activity, not continuously changing live exchange state.

If a caller later needs current open-position or equity data, the application can request it from Hyperliquid on demand rather than persisting it as part of this module's primary history model.

## Reconciliation Objective

The goal is to reconcile all account activity for the configured Hyperliquid account.

For the first version, exact reconciliation should mean:

1. all relevant exchange events are captured locally
2. local events are stored exactly once
3. local numeric values match exchange values
4. local derived totals over time windows match exchange-visible history

This should apply to:

- fills
- funding
- deposits
- withdrawals
- transfers relevant to the account
- fees

## Startup and Runtime Flow

### Startup

On startup, for each configured account:

1. acquire a per-account sync lock
2. load prior sync state
3. perform reconciliation before entering live mode
4. if reconciliation fails, do not proceed for that account
5. if reconciliation succeeds, begin periodic live maintenance loops

Important nuance:

- first startup for a new account should perform an initial historical bootstrap
- later startups should reconcile from the last durable checkpoint forward, using an overlap safety window

The system should not depend on doing a full all-history replay on every restart.

### Runtime

After successful startup reconciliation:

- scheduled HTTP `/info` polls fetch recent windows for each stream and backfill any gaps
- reconciliation jobs verify recent windows continuously

For the first version there is no websocket. The correctness path for every stream is HTTP overlap polling via the app-owned raw `/info` client. A websocket fast-path for live fills/order updates may be added later as a latency optimization, never as a correctness dependency.

## Data Sources

### Historical / Ongoing API Sync

The module uses `/info` endpoints for bootstrap, ongoing maintenance, and gap repair. These are app-owned raw HTTP calls because NT does not expose them.

Endpoints for V1:

- `userFillsByTime` — fill history backfill, ongoing poll, and repair
- `userFunding` — user funding history
- `nonUserFundingUpdates` — non-funding ledger history (deposits, withdrawals, transfers)
- `historicalOrders` — historical order state

Notes:

- `/info` is a public, unauthenticated `POST`; the history endpoints require no signing
- the calls use `reqwest` (or `hypersdk`'s client)
- `userFillsByTime` has retention limits
- the time-range endpoints support windowed pagination
- the module relies on continuous polling plus overlap-safe repair rather than deep API recovery

### WebSocket Live Feed (deferred)

Deferred out of journal v1. When added later, it would only be a fast-path for live fills/order updates. It cannot be a journal correctness source because the NT WS handler drops funding/ledger events and emits NT domain types rather than raw exchange JSON.

### HTTP Path Is The Truth Path

All journal correctness comes from the HTTP `/info` polling path:

- each stream is polled on a short interval with an overlap window
- backfills anything missed during downtime
- provides an idempotent, restart-safe correctness check

## Execution Intent and Order Submission

This module should own the canonical bridge between:

- memory analysis
- submitted order intent
- exchange-confirmed activity

That means the Hyperliquid subsystem owns the local order journal.

This should not be thought of as a transparent reverse proxy for the entire Hyperliquid API.

It should be thought of as an internal `execution gateway`.

The authoritative, concrete build target for this gateway is
`.opencode/plans/order-submission-system.md`. The sections below describe
the agreed design direction; the plan file has the exact schema, module
layout, and step order.

The implementation is now complete in `src/hyperliquid/orders/`:

- `rounding.rs` — price/size rounding (conservative direction).
- `model.rs` — request/response DTOs + validation.
- `store.rs` — DB access for `hyperliquid.orders` and
  `hyperliquid.order_events`. All agent-facing queries are scoped by
  `agent_key`; the WebSocket path uses account-scoped lookups.
- `gateway.rs` — orchestration: validates the request, looks up the
  instrument, rounds, inserts pending rows, calls the exchange
  (always HTTP, never WebSocket), maps the response back to each
  leg. `ExchangeClient` is a trait so the orchestration is
  unit-tested offline with a fake.
- `reconcile.rs` — periodic reconciler: pulls live `open_orders`
  and positions, merges observed exchange status into the local
  row, resolves `pending_submission` orders that have vanished
  from the exchange, and auto-cancels orphaned reduce-only TP/SL
  legs whose underlying position is flat.

The default market-order slippage is `DEFAULT_MARKET_SLIPPAGE_BPS = 50`
(0.5%) and is applied symmetrically to the mid price: buys widen UP,
sells widen DOWN. The widened price is then rounded to the
instrument's `price_decimals` in the conservative direction (buy → down,
sell → up).

Submission always uses `OrderGrouping::Na`; the entry and any
attached TP/SL legs go in the same `BatchOrder`. Auto-cancel of
orphaned reduce-only legs is the reconciler's job, not the
exchange's.

### Preferred Direction

Preferred direction:

- agents submit private trading actions through an internal Hyperliquid execution gateway
- the app authenticates the caller via the agent API key and resolves `agent_key` before execution logic runs
- the gateway records a local order row (status `pending_submission`) before any exchange call
- the gateway then forwards the request to Hyperliquid over the signed HTTP `/exchange` client
- later exchange events (HTTP response, live WebSocket order updates, and journal polling) reconcile back to those local records

This gateway lives inside the same application binary.

It does not need to be a separate deployable service.

### Submission Transport: HTTP Only

Order placement and cancellation go over the `hypersdk` **signed HTTP**
client (`client.place(...)`, `client.cancel(...)`).

They do **not** go over the WebSocket. The `hypersdk` 0.2.x WebSocket
client only supports `subscribe` / `unsubscribe`; it exposes no `post`
action method. The WebSocket remains a read-only live feed.

Signing uses the agent's Hyperliquid private key, which the app decrypts
on demand from `agents.hyperliquid_private_key_ciphertext` and
loads into an `alloy` `PrivateKeySigner`. Agents never see the key.

### What The Gateway Should Cover

The gateway should primarily own private write actions such as:

- place order (limit, market)
- place reduce-only take-profit and stop-loss trigger orders
- cancel order(s)
- cancel-all (optionally scoped to one symbol)
- batch place and batch cancel

Modify is not in the first version. It can be added later.

It does not need to mirror every public Hyperliquid read endpoint.

Historical reads should mostly come from:

- the local Postgres journal
- direct backend calls to Hyperliquid when current live state is needed

### Why A Gateway Is Better Than Direct Exchange Access

Benefits:

- preserves a reliable local audit trail before the exchange call happens
- links each order to `agent_key` and (later) memory records
- avoids giving every agent raw execution credentials by default
- allows later policy enforcement such as account, instrument, or size restrictions
- gives a clean place to enforce idempotency and retry behavior

### Client Order IDs (`cloid`)

The gateway **always generates a client order id (`cloid`) for every
order** before submission.

- `cloid` is a 128-bit value the app generates and stores locally.
- `oid` is the exchange-assigned order id (`u64`) returned once the order rests or fills.

The `cloid` lets the app correlate its local row to the exchange order
even before an `oid` is known, and survives network timeouts. Agents
cancel using the `oid` returned from "list open orders"; the gateway can
internally cancel by either `oid` or `cloid`.

### Order Lifecycle and Status Tracking

Order submission is not a binary success/failure. The full lifecycle is
captured so the system can always reconstruct what happened.

Latest status lives on the `hyperliquid.orders` row. Status values:

- `pending_submission` — local row written, not yet sent
- `submitted` — sent, awaiting a definitive result
- `resting` — accepted and resting on the book (has an `oid`)
- `partially_filled`
- `filled`
- `canceled`
- `rejected` — rejected by the exchange
- `error` — local/transport error before a known exchange outcome
- `unknown` — timed out; must be reconciled against exchange history

Every status transition is also appended to an immutable
`hyperliquid.order_events` child table, so the entire lifecycle
(submitted → accepted/resting → partial fill → filled → canceled, etc.)
is preserved, not just the latest state.

Status transitions come from three sources, all writing `order_events`:

1. the synchronous HTTP response to `place` / `cancel` (`source = http_response`)
2. the live WebSocket `OrderUpdates` subscription (`source = ws_order_update`) — already subscribed by `live_ws`; this feature wires the previously-ignored handler into order tracking
3. the order reconciliation worker (`source = reconcile`), using `open_orders`, `historicalOrders`, and `trade_fills`

`order_events` is idempotent: dedup on `(order_id, status, status_timestamp, source)`.

### Take-Profit and Stop-Loss Orders

TP/SL are modeled as **reduce-only trigger orders**:

- take-profit = a take-profit limit trigger (`TpSl::Tp`, `is_market = false`)
- stop-loss = a stop-market trigger (`TpSl::Sl`, `is_market = true`)

Multiple TP and multiple SL orders may be placed for one position. The
gateway groups an entry order and its TP/SL legs under a local
`group_id` so they can be associated and managed together.

Hyperliquid does **not** automatically cancel orphaned reduce-only
trigger orders when a position is closed by other means. Auto-cancelling
the leftover legs of a `group_id` once its position is flat is the
responsibility of the order reconciliation worker, not the exchange.

### Single Orders Table (revised direction)

Earlier drafts of this doc proposed two tables (`execution_intents` and
`submitted_orders`). The agreed direction is simpler:

- one `hyperliquid.orders` table records every submitted order (including
  ones later cancelled), with the latest status, the rounded vs requested
  price/size, `cloid`, `exchange_oid`, `group_id`, and request/response payloads
- one append-only `hyperliquid.order_events` table records the lifecycle

`memory_record_ids` is carried on `hyperliquid.orders` as a JSON column so
order→memory linkage can be added later without a migration.

### Price and Size Rounding

Hyperliquid requires `limit_px` and `sz` to be aligned to each
instrument's tick and lot size before signing. `hypersdk`'s `place` does
not auto-round.

The gateway rounds price and size server-side using
`price_decimals` / `size_decimals` from `hyperliquid.instruments` before
signing, in a conservative direction. Both the requested and rounded
values are stored on the order row. Unknown symbols are rejected.

### Why This Module Should Own It

If orders are submitted directly to Hyperliquid without a canonical local
order record, the system loses part of the audit chain.

The system then cannot reliably distinguish between:

- no trade was intended
- a trade was intended but never submitted
- a trade was submitted but not filled
- a trade filled differently than intended

Because of that, order submission lives here rather than in the memory schema.

### Fallback Direction

If an agent is ever allowed to call Hyperliquid directly, it must also
write the matching local order record.

That is possible, but it is less safe than the gateway pattern.

## Instrument Reference Sync

This module should also sync Hyperliquid instrument and market-reference data into Postgres.

This is needed because Hyperliquid payloads may refer to markets using:

- perp coin names
- spot `@index` identifiers
- HIP-3 prefixed symbols

Without a local reference table, symbol-level summaries and evaluation joins will become brittle.

The Hyperliquid subsystem should own this normalization layer.

It should provide a canonical mapping from exchange identifiers to the local instrument key used elsewhere in the system.

## Internal Module Responsibilities

The Hyperliquid subsystem should likely be split internally into these parts:

### 1. Historical Sync

Responsibilities:

- initial bootstrap
- gap repair after downtime
- periodic historical re-check of recent windows

### 2. Live Polling

Responsibilities:

- maintain recent-window polling loops
- insert live account-history updates into local storage
- keep overlap windows safe across restarts
- optionally trigger faster fill refresh when recent activity is detected

### 3. Normalization Layer

Responsibilities:

- convert raw Hyperliquid payloads into canonical event records
- assign deterministic event keys
- map event types into journal rows and typed tables

### 4. Reconciliation Engine

Responsibilities:

- verify expected windows
- detect missing events
- detect duplicate events
- compare derived local totals against exchange history

### 5. Query / Projection Layer

Responsibilities:

- provide efficient summary tables or views
- support agent evaluation queries
- support operator UI queries

## Database Direction

This module should use its own Postgres schema.

Recommended schema name:

- `hyperliquid`

The schema should support multiple accounts from the start.

Even if initial transaction volume is low, the schema should still be designed for:

- exactness
- idempotency
- efficient time-window queries
- per-account isolation keyed by `account_address` + `environment`

## Proposed Schema Shape

The first version should use typed event tables as the canonical storage, plus a unified timeline view:

1. canonical typed event tables:
   - `trade_fills`
   - `funding_events`
   - `ledger_events`
   - `historical_orders`
2. unified timeline view:
   - `account_timeline` (UNION ALL over the three cash/PnL event tables; `historical_orders` is excluded because it is order state, not a cash/PnL event)

### Core Tables

For the first implementation, the actual build target should stay smaller than the broader future shape described elsewhere in this file.

Phase 1 priority tables:

- `hyperliquid.instruments`
- `hyperliquid.sync_state`
- `hyperliquid.trade_fills`
- `hyperliquid.funding_events`
- `hyperliquid.ledger_events`
- `hyperliquid.historical_orders`

Phase 1 views:

- `hyperliquid.account_timeline`

See `.opencode/plans/hyperliquid-account-history-journal.md` for the authoritative phase-1 column lists, identifier semantics (`hash` vs `tx_hash`, `fee` vs `fee_usdc`), the instrument-resolution null policy, the `environment` CHECK constraint, and the `raw_http` module spec.

Execution gateway tables (added by the order submission feature):

- `hyperliquid.orders` — one row per submitted order, latest status
- `hyperliquid.order_events` — append-only order lifecycle transitions

See `.opencode/plans/order-submission-system.md` for the authoritative
column lists and module layout for these two tables. They supersede the
older `execution_intents` / `submitted_orders` two-table idea described
further down in this document.

Future extension tables, not required for the first implementation:

- `hyperliquid.accounts`
- `hyperliquid.activity_events`
- `hyperliquid.sync_runs`
- `hyperliquid.ws_sessions`
- `hyperliquid.reconcile_runs`
- `hyperliquid.reconcile_issues`

Note: the older `hyperliquid.execution_intents` and
`hyperliquid.submitted_orders` table ideas (described later in this
document) are superseded by the single `hyperliquid.orders` +
`hyperliquid.order_events` design above.

### Derived Tables or Views

- `hyperliquid.daily_account_summary`
- `hyperliquid.daily_symbol_summary`
- `hyperliquid.realized_pnl_summary`
- `hyperliquid.fee_summary`
- `hyperliquid.funding_summary`

## Core Table Ideas

### Account Identity In Phase 1

For the first implementation, the journal does not need a dedicated `accounts` table.

Instead, key rows directly by:

- `account_address`
- `environment`

An `accounts` table can be added later if the app needs a richer account registry, but phase 1 intentionally avoids introducing a UUID `account_id` to keep the journal simple and to align with the natural exchange identity.

### `hyperliquid.instruments`

Purpose:

- local reference table for Hyperliquid market and asset identifiers
- canonical symbol normalization for summaries, agents, and evaluation

Suggested fields:

- `instrument_id`

The `instrument_id` is the Hyperliquid coin name (e.g. `BTC`).

- `name`
- `market_type`
- `base_asset`
- `quote_asset`
- `settlement_asset`
- `asset_index`
- `price_decimals`
- `size_decimals`
- `lot_size`
- `max_leverage`
- `is_hip3`
- `active`
- `created_at`
- `updated_at`

### `hyperliquid.sync_state`

Purpose:

- durable progress tracking per account and stream
- startup reconciliation checkpoints

Suggested fields:

- `account_address`
- `environment`
- `stream_name`
- `last_synced_at`
- `last_event_time`
- `last_event_key`
- `status`
- `metadata`

### `hyperliquid.execution_intents`

> SUPERSEDED: the `execution_intents` + `submitted_orders` two-table model
> described in this and the next subsection is no longer the plan. It is
> replaced by a single `hyperliquid.orders` table plus an append-only
> `hyperliquid.order_events` table. See the "Execution Intent and Order
> Submission" section above and `.opencode/plans/order-submission-system.md`.
> These subsections are kept only for historical context.

Purpose:

- `id`
- `created_at`
- `agent_key`
- `account_address`
- `environment`
- `intent_type`
- `side`
- `symbol`
- `instrument_id`
- `requested_size`
- `requested_price`
- `time_in_force`
- `reduce_only`
- `client_order_id`
- `memory_record_ids`
- `reason_codes`
- `status`
- `payload`

Suggested status examples:

- `pending_submission`
- `submitted`
- `canceled_before_submit`
- `unknown_pending_reconcile`

### `hyperliquid.submitted_orders`

Purpose:

- local record of what was actually sent to Hyperliquid by the execution gateway
- track the submission side of the exchange audit chain

Suggested fields:

- `id`
- `execution_intent_id`
- `account_address`
- `environment`
- `submitted_at`
- `client_order_id`
- `exchange_order_id`
- `submission_status`
- `request_payload`
- `response_payload`
- `metadata`

Suggested submission status examples:

- `accepted_http`
- `http_error`
- `exchange_rejected`
- `timed_out_pending_reconcile`

### Unified timeline view

Purpose:

- combine `trade_fills`, `funding_events`, and `ledger_events` into a single timeline
- support queries that need to see everything for an account in time order

Suggested approach:

- create `hyperliquid.account_timeline` as a `UNION ALL` view over the three typed tables
- expose common columns: `account_address`, `environment`, `event_time`, `event_type`, `source_stream`, `instrument_id`, `asset`, `symbol`, `fee_usdc`, `realized_pnl_usdc`, `payload`, `ingest_source`, `inserted_at`, `event_category`
- expose `hash` where available (fills and ledger); synthesize a composite text id for funding

### `hyperliquid.activity_events` (deferred)

This table is intentionally not used in v1.

Rationale:

- v1 treats each typed table (`trade_fills`, `funding_events`, `ledger_events`) as canonical
- common journal columns are carried directly on each typed table
- the `account_timeline` view provides the unified timeline that `activity_events` would otherwise provide
- this avoids duplicating common fields between a parent journal table and typed child tables

If a future phase needs a materialized universal event table, add `activity_events` then and backfill it from the typed tables.

### `hyperliquid.trade_fills`

Purpose:

- canonical fill storage
- detailed querying by symbol, side, fee, and realized outcome

Suggested fields:

- `hash` (PRIMARY KEY)
- `account_address`
- `environment`
- `event_time`
- `event_type`
- `source_stream`
- `instrument_id`
- `asset`
- `symbol`
- `fee_usdc`
- `realized_pnl_usdc`
- `fill_time`
- `direction`
- `side`
- `price`
- `size`
- `trade_value`
- `order_id`
- `trade_id`
- `start_position`
- `fee`
- `fee_token`
- `builder_fee`
- `crossed`
- `tx_hash`
- `payload`
- `ingest_source`
- `inserted_at`

### `hyperliquid.funding_events`

Purpose:

- canonical funding history

Suggested fields:

- `account_address`
- `environment`
- `instrument_id`
- `event_time`
- `event_type`
- `source_stream`
- `asset`
- `symbol`
- `fee_usdc`
- `realized_pnl_usdc`
- `usdc`
- `position_size`
- `funding_rate`
- `hash`
- `payload`
- `ingest_source`
- `inserted_at`

Primary key: (`account_address`, `environment`, `instrument_id`, `event_time`)

### `hyperliquid.ledger_events`

Purpose:

- canonical non-funding ledger delta storage

Suggested fields:

- `hash` (PRIMARY KEY)
- `account_address`
- `environment`
- `event_time`
- `event_type`
- `source_stream`
- `instrument_id`
- `asset`
- `symbol`
- `fee_usdc`
- `realized_pnl_usdc`
- `ledger_type`
- `usdc`
- `token`
- `amount`
- `fee`
- `source_user`
- `destination_user`
- `tx_hash`
- `details`
- `payload`
- `ingest_source`
- `inserted_at`

## Draft Relational Schema

This section turns the table ideas above into a more concrete first-pass schema draft.

Important note:

- parts of the long-form draft below describe a broader future schema shape
- the current phase-1 direction is the smaller table set listed above
- phase 1 should not be blocked on introducing `hyperliquid.accounts`, `activity_events`, `sync_runs`, `ws_sessions`, or reconcile tables

The goal is not to lock every column forever.

The goal is to define a strong enough starting structure for:

- exact history capture
- idempotent ingestion
- execution-intent tracking
- efficient evaluation queries

### General Conventions

Use these general rules:

- `UUID` primary keys for internal rows
- `TIMESTAMPTZ` for all timestamps
- `TEXT` plus `CHECK` constraints for status-like fields in the first version
- `NUMERIC` for financial values
- `JSONB` for flexible payloads and metadata
- foreign keys where subsystem ownership is clear

Suggested common columns:

- `created_at TIMESTAMPTZ NOT NULL`
- `updated_at TIMESTAMPTZ NOT NULL` where rows can change over time
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

### 1. `hyperliquid.accounts`

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `updated_at TIMESTAMPTZ NOT NULL`
- `status TEXT NOT NULL`
- `account_address TEXT NOT NULL`
- `account_label TEXT`
- `environment TEXT NOT NULL`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:

- `UNIQUE (account_address, environment)`
- `CHECK (status IN ('active', 'disabled', 'archived'))`

### 2. `hyperliquid.instruments`

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `updated_at TIMESTAMPTZ NOT NULL`
- `status TEXT NOT NULL`
- `exchange_symbol TEXT NOT NULL`
- `canonical_symbol TEXT NOT NULL`
- `market_type TEXT NOT NULL`
- `asset_id TEXT`
- `dex TEXT`
- `base_asset TEXT`
- `quote_asset TEXT`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:

- unique index on `(exchange_symbol, COALESCE(dex, ''))`
- `CHECK (market_type IN ('perp', 'spot', 'hip3_perp', 'other'))`

### 3. `hyperliquid.sync_state`

Suggested columns:

- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `stream_name TEXT NOT NULL`
- `last_synced_at TIMESTAMPTZ`
- `last_event_time TIMESTAMPTZ`
- `last_event_key TEXT`
- `last_successful_reconcile_at TIMESTAMPTZ`
- `status TEXT NOT NULL`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested keys:

- `PRIMARY KEY (account_address, environment, stream_name)`

Suggested constraints:



### 4. `hyperliquid.sync_runs`

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `run_type TEXT NOT NULL`
- `stream_name TEXT`
- `window_start TIMESTAMPTZ`
- `window_end TIMESTAMPTZ`
- `status TEXT NOT NULL`
- `rows_seen INTEGER`
- `rows_inserted INTEGER`
- `rows_updated INTEGER`
- `error_text TEXT`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:



### 5. `hyperliquid.ws_sessions`

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `started_at TIMESTAMPTZ NOT NULL`
- `ended_at TIMESTAMPTZ`
- `status TEXT NOT NULL`
- `disconnect_reason TEXT`
- `last_message_at TIMESTAMPTZ`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:



### 6. `hyperliquid.activity_events`

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `updated_at TIMESTAMPTZ NOT NULL`
- `agent_key TEXT NOT NULL`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `intent_type TEXT NOT NULL`
- `side TEXT`
- `symbol TEXT NOT NULL`
- `instrument_id UUID`
- `requested_size NUMERIC(38, 18)`
- `requested_price NUMERIC(38, 18)`
- `time_in_force TEXT`
- `reduce_only BOOLEAN NOT NULL DEFAULT false`
- `client_order_id TEXT`
- `memory_record_ids JSONB NOT NULL DEFAULT '[]'::jsonb`
- `reason_codes TEXT[] NOT NULL DEFAULT '{}'`
- `status TEXT NOT NULL`
- `payload JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:

- if the agents subsystem is present, `FOREIGN KEY (agent_key) REFERENCES agents(agent_key)`

- `FOREIGN KEY (instrument_id) REFERENCES hyperliquid.instruments(id)`
- `UNIQUE (account_address, environment, client_order_id)` where `client_order_id` is not null

Suggested status values:

- `pending_submission`
- `submitted`
- `canceled_before_submit`
- `unknown_pending_reconcile`

### 8. `hyperliquid.submitted_orders`

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `execution_intent_id UUID NOT NULL`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `submitted_at TIMESTAMPTZ NOT NULL`
- `client_order_id TEXT`
- `exchange_order_id TEXT`
- `submission_status TEXT NOT NULL`
- `request_payload JSONB NOT NULL`
- `response_payload JSONB`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:

- `FOREIGN KEY (execution_intent_id) REFERENCES hyperliquid.execution_intents(id)`

- `UNIQUE (account_address, environment, exchange_order_id)` where `exchange_order_id` is not null

Suggested status values:

- `accepted_http`
- `http_error`
- `exchange_rejected`
- `timed_out_pending_reconcile`

### 9. `hyperliquid.activity_events`

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `event_time TIMESTAMPTZ NOT NULL`
- `event_type TEXT NOT NULL`
- `instrument_id UUID`
- `source_stream TEXT NOT NULL`
- `source_hash TEXT`
- `source_event_key TEXT NOT NULL`
- `asset TEXT`
- `symbol TEXT`
- `usdc_delta NUMERIC(38, 18)`
- `fee_usdc NUMERIC(38, 18)`
- `realized_pnl_usdc NUMERIC(38, 18)`
- `execution_intent_id UUID`
- `submitted_order_id UUID`
- `payload JSONB NOT NULL DEFAULT '{}'::jsonb`
- `ingest_source TEXT NOT NULL`
- `inserted_at TIMESTAMPTZ NOT NULL`

Suggested constraints:


- `FOREIGN KEY (instrument_id) REFERENCES hyperliquid.instruments(id)`
- `FOREIGN KEY (execution_intent_id) REFERENCES hyperliquid.execution_intents(id)`
- `FOREIGN KEY (submitted_order_id) REFERENCES hyperliquid.submitted_orders(id)`
- `UNIQUE (account_address, environment, source_stream, source_event_key)`

### 9. `hyperliquid.trade_fills`

Suggested columns:

- `hash TEXT PRIMARY KEY`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `instrument_id TEXT NOT NULL`
- `fill_time TIMESTAMPTZ NOT NULL`
- `side TEXT NOT NULL`
- `direction TEXT NOT NULL`
- `price NUMERIC(38, 18) NOT NULL`
- `size NUMERIC(38, 18) NOT NULL`
- `trade_value NUMERIC(38, 18)`
- `order_id TEXT`
- `trade_id TEXT NOT NULL`
- `start_position NUMERIC(38, 18)`
- `realized_pnl_usdc NUMERIC(38, 18)`
- `fee NUMERIC(38, 18)`
- `fee_token TEXT`
- `builder_fee NUMERIC(38, 18)`
- `crossed BOOLEAN`
- `tx_hash TEXT`
- `asset TEXT`
- `symbol TEXT`
- `event_type TEXT NOT NULL DEFAULT 'fill'`
- `source_stream TEXT NOT NULL`
- `fee_usdc NUMERIC(38, 18)`
- `payload JSONB NOT NULL DEFAULT '{}'::jsonb`
- `ingest_source TEXT NOT NULL`
- `inserted_at TIMESTAMPTZ NOT NULL`

Suggested constraints:

- `FOREIGN KEY (instrument_id) REFERENCES hyperliquid.instruments(instrument_id)`
- `UNIQUE (account_address, environment, source_stream, hash)`

### 10. `hyperliquid.funding_events`

Suggested columns:

- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `instrument_id TEXT NOT NULL`
- `event_time TIMESTAMPTZ NOT NULL`
- `usdc NUMERIC(38, 18) NOT NULL`
- `position_size NUMERIC(38, 18)`
- `funding_rate NUMERIC(38, 18)`
- `hash TEXT`
- `asset TEXT`
- `symbol TEXT`
- `event_type TEXT NOT NULL DEFAULT 'funding'`
- `source_stream TEXT NOT NULL`
- `fee_usdc NUMERIC(38, 18)`
- `realized_pnl_usdc NUMERIC(38, 18)`
- `payload JSONB NOT NULL DEFAULT '{}'::jsonb`
- `ingest_source TEXT NOT NULL`
- `inserted_at TIMESTAMPTZ NOT NULL`

Suggested constraints:

- `PRIMARY KEY (account_address, environment, instrument_id, event_time)`
- `FOREIGN KEY (instrument_id) REFERENCES hyperliquid.instruments(instrument_id)`
- `UNIQUE (account_address, environment, source_stream, instrument_id, event_time)`

### 11. `hyperliquid.ledger_events`

Suggested columns:

- `hash TEXT PRIMARY KEY`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `instrument_id TEXT`
- `event_time TIMESTAMPTZ NOT NULL`
- `ledger_type TEXT NOT NULL`
- `usdc NUMERIC(38, 18)`
- `token TEXT`
- `amount NUMERIC(38, 18)`
- `fee NUMERIC(38, 18)`
- `source_user TEXT`
- `destination_user TEXT`
- `tx_hash TEXT`
- `asset TEXT`
- `symbol TEXT`
- `event_type TEXT NOT NULL`
- `source_stream TEXT NOT NULL`
- `fee_usdc NUMERIC(38, 18)`
- `realized_pnl_usdc NUMERIC(38, 18)`
- `details JSONB NOT NULL DEFAULT '{}'::jsonb`
- `payload JSONB NOT NULL DEFAULT '{}'::jsonb`
- `ingest_source TEXT NOT NULL`
- `inserted_at TIMESTAMPTZ NOT NULL`

Suggested constraints:

- `FOREIGN KEY (instrument_id) REFERENCES hyperliquid.instruments(instrument_id)`
- `UNIQUE (account_address, environment, source_stream, hash)`


### 13. `hyperliquid.reconcile_runs`

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `window_start TIMESTAMPTZ NOT NULL`
- `window_end TIMESTAMPTZ NOT NULL`
- `status TEXT NOT NULL`
- `summary TEXT`
- `details JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:



### 14. `hyperliquid.reconcile_issues`

Suggested columns:

- `id UUID PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `reconcile_run_id UUID NOT NULL`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `issue_type TEXT NOT NULL`
- `severity TEXT NOT NULL`
- `event_time TIMESTAMPTZ`
- `summary TEXT NOT NULL`
- `details JSONB NOT NULL DEFAULT '{}'::jsonb`
- `resolved_at TIMESTAMPTZ`

Suggested constraints:

- `FOREIGN KEY (reconcile_run_id) REFERENCES hyperliquid.reconcile_runs(id)`


### Suggested Initial Indexes

For `hyperliquid.accounts`:

- `(account_address, environment)` unique

For `hyperliquid.instruments`:

- `(canonical_symbol)`
- `(exchange_symbol, dex)` unique

For `hyperliquid.sync_state`:

- primary key `(account_address, environment, stream_name)`

For `hyperliquid.execution_intents`:

- `(agent_key, created_at DESC)`
- `(account_address, environment, status, created_at DESC)`
- `(account_address, environment, client_order_id)` unique where not null

For `hyperliquid.submitted_orders`:

- `(execution_intent_id)`
- `(account_address, environment, submitted_at DESC)`
- `(account_address, environment, exchange_order_id)` unique where not null

For `hyperliquid.activity_events`:

- `(account_address, environment, event_time DESC)`
- `(account_address, environment, event_type, event_time DESC)`
- `(account_address, environment, symbol, event_time DESC)`
- `(execution_intent_id)`
- `(submitted_order_id)`
- `(account_address, environment, source_stream, source_event_key)` unique

For `hyperliquid.trade_fills`:

- `(account_address, environment, fill_time DESC)`
- `(account_address, environment, instrument_id, fill_time DESC)`
- `(trade_id)`

For `hyperliquid.funding_events`:

- `(account_address, environment, event_time DESC)`
- `(account_address, environment, instrument_id, event_time DESC)`

For `hyperliquid.ledger_events`:

- `(account_address, environment, event_time DESC)`
- `(account_address, environment, ledger_type, event_time DESC)`

For `hyperliquid.reconcile_runs`:

- `(account_address, environment, window_start DESC, window_end DESC)`

For `hyperliquid.reconcile_issues`:

- `(account_address, environment, created_at DESC)`
- `(reconcile_run_id)`

### Example Order Lifecycle

1. Agent decides to trade and (optionally) references one or more memory records.
2. The execution gateway rounds price/size, generates a `cloid`, and writes a `hyperliquid.orders` row with status `pending_submission`.
3. The gateway submits the signed order over HTTP and records the response: it updates the `orders` row status (`resting` / `filled` / `rejected` / `error`) and appends a `hyperliquid.order_events` row (`source = http_response`).
4. Live WebSocket `OrderUpdates` stream further transitions (partial fill, fill, cancel) into `order_events` (`source = ws_order_update`) and update the `orders` latest status.
5. Historical or recent-window polling later produces exchange-confirmed events; normalization writes typed rows like `trade_fills`, `funding_events`, and `ledger_events`.
6. The order reconciliation worker links the final exchange truth back to the local `orders` row (`source = reconcile`), resolves `unknown` orders, and cancels orphaned reduce-only TP/SL legs once their position is flat.

## Idempotency and Event Identity

The module must be idempotent.

Every ingest path should be able to safely re-process overlapping windows or duplicate live refreshes.

Use deterministic source-specific event keys wherever possible.

Examples:

- fills keyed by the exchange tx hash
- funding keyed by account + environment + instrument_id + time
- ledger updates keyed by the exchange tx hash

The exact key format can be decided later, but idempotent upsert behavior is a hard requirement.

## Reconciliation Strategy

The module should reconcile by time windows, not just by row existence.

Examples of reconciliation checks:

- fill count over a window
- total fees over a window
- total funding over a window
- total deposits over a window
- total withdrawals over a window
- total realized PnL over a window

The engine should record:

- successful reconcile windows
- mismatches
- repair attempts
- unresolved issues

## Query Requirements

This module exists partly to support the memory system and later agent evaluation.

It should make queries like these easy:

- realized PnL for a symbol over a date range
- total fees paid over a date range
- total funding paid / received over a date range
- fills for a symbol over a date range
- net deposits / withdrawals over a date range
- activity timeline for one account

That means indexes and summary structures should favor:

- `account_address`
- `environment`
- `event_time`
- `event_type`
- `symbol`

## Single Binary Architecture

The current direction is to run everything in one binary.

That is acceptable as long as internal module boundaries stay clean.

The one binary should host:

- memory API
- web UI
- Hyperliquid account sync and reconciliation
- Hyperliquid polling and repair workers

Suggested internal runtime tasks:

1. HTTP server
2. Hyperliquid supervisor
3. per-account polling worker
4. periodic historical sync worker
5. periodic reconciliation worker

This keeps deployment simple while preserving subsystem separation.

## Process Philosophy

Even inside one binary, the Hyperliquid subsystem should behave like a supervised module with its own responsibilities.

Recommended internal code split:

- `hyperliquid::accounts`
- `hyperliquid::reference_data`
- `hyperliquid::orders`
- `hyperliquid::sync`
- `hyperliquid::poll`
- `hyperliquid::normalize`
- `hyperliquid::reconcile`
- `hyperliquid::queries`

## Deferred Features

These are intentionally deferred for now:

- webhook delivery to registered agent endpoints
- agent event registry
- vault activity support
- staking support
- deep archival import from node or S3 data
- open position tracking as a primary concern

Webhook delivery may be added later as a consumer of internal events emitted by this module.

## Important Constraints and Assumptions

- transaction volume is expected to be low
- the sync process should be running whenever trades are being made
- startup reconciliation is required before live operation
- after startup, polling becomes the primary live maintenance path
- periodic repair syncs are still needed in case of missed windows or temporary API failures

## Open Questions

1. Should V1 include sub-account transfers and spot transfers whenever they touch the tracked account, or should those be deferred?
2. ~~Should `historicalOrders` be stored in V1?~~ Resolved: yes. V1 stores it in a dedicated `hyperliquid.historical_orders` table (order state, excluded from `account_timeline`).
3. Should summary data be implemented as materialized views, normal views, or physical rollup tables?
4. How should account configuration be split between the agents registry, app config, and encrypted secret storage?

## Summary

The Hyperliquid module should be a separate subsystem with its own `hyperliquid` Postgres schema.

Its main job is to maintain an exact local account activity journal by combining:

- startup historical reconciliation
- live polling and repair
- periodic gap repair
- ongoing reconciliation checks

It should support multiple accounts in schema design, ignore vault and staking concerns for now, and run inside the same application binary as the memory subsystem and web UI.
