# Architecture

The current application architecture is:

- one Rust binary
- one Postgres database
- one server-rendered web UI and internal JSON API surface
- background tasks for Hyperliquid account monitoring and OpenCode schedule dispatch

Core subsystems:

- `agents` for registry, runtimes, prompts, and instrument ownership
- `memory` for stored analysis and execution context
- `hyperliquid` for account state, history, and execution
- `agentic` for OpenCode job scheduling, dispatch, and run tracking
- OpenCode job dispatch injects initial agent/job context into the command prompt

Current backend support:

- only `opencode` is supported today
- the runtime/backend schema remains generic so future backend kinds can be added later

## Rust Focus

Rust is still a strong option because:

- there is already more familiarity with Rust than Python
- the app will likely benefit from strong typing and explicit design
- a Rust backend can cleanly serve both HTML and JSON
- background worker processes fit naturally with `tokio`

The main downside is that Hyperliquid support in Rust is less clearly official than Python.

## Main Rust Web Options

### Option 1: `actix-web`

Why consider it:

- mature and proven
- good performance
- straightforward routing and handler model
- works well for server-rendered HTML and JSON APIs

Suggested stack:

- `actix-web`
- `askama` for templates
- `sqlx` for Postgres
- `sqlx migrate` for migrations
- `tokio` for async runtime and worker tasks
- `serde` for JSON
- `tracing` for logging and observability

This is a good fit if the implementation should stay close to a traditional web app plus worker model.

### Option 2: `axum`

Why consider it:

- strong alignment with the broader `tokio` / `tower` ecosystem
- clean composition model
- common choice for modern Rust services
- good fit for shared middleware and internal service architecture

Suggested stack:

- `axum`
- `askama` for templates
- `sqlx` for Postgres
- `sqlx migrate` for migrations
- `tokio` for async runtime and worker tasks
- `serde` for JSON
- `tracing` for logging and observability

This may be the cleaner default if starting fresh with a Rust service architecture.

## Hyperliquid Integration (`hypersdk`)

Current direction:

- use the `hypersdk` Rust crate as the sole Hyperliquid SDK
- `hypersdk` covers instrument loading, live account state, the live WebSocket, and signing/order submission
- the app owns the account-history `/info` HTTP client for fills, funding, ledger, and historical orders
- execution is app-driven through an internal execution gateway

The app owns:

- account-history journaling via an app-owned raw HTTP `/info` client
- execution intent tracking (later)
- order submission workflow (later, using `hypersdk` signing/submission)
- reconciliation logic

`hypersdk` is the integration surface; the app remains the orchestration layer.

## Rust UI Direction

The frontend does not need to be heavily optimized around any specific framework yet.

Reasonable options:

- server-rendered templates only
- server-rendered templates with small amounts of vanilla JS
- server-rendered templates with `htmx` later if useful

If choosing Rust, the UI should likely stay simple and server-rendered at first.

Template engine candidates:

- `askama`
- `maud`

Current leaning:

- `askama`

because it is simple, common, and a good fit for page templates plus partials.

## Database / Persistence Direction

Current DB direction:

- `Postgres`
- dedicated `memory` schema for the memory subsystem
- dedicated `agents` schema for the agent registry subsystem
- dedicated `hyperliquid` schema for account activity and reconciliation
- likely additional schemas later for account history and trading state

Rust DB tooling direction:

- `sqlx`
- `sqlx migrate`

Why:

- good Postgres support
- compile-time checked queries are attractive for this kind of system
- migration workflow is simple enough

## Migration Strategy

Current direction:

- use one normal shared migrations directory
- run one migration command for the whole app
- keep separate Postgres schemas per subsystem inside the same database
- allow cross-schema foreign keys where ownership is clear and useful

Recommended shape:

```text
migrations/
  0001_agents.sql
  0002_memory.sql
  0003_hyperliquid.sql
  0004_memory_active_state.sql
  0005_hyperliquid_execution_gateway.sql
```

Why this is the current preference:

- simpler than building a custom per-module migration runner
- works naturally with normal migration tooling like `sqlx migrate`
- still preserves clean module ownership through separate schemas
- makes cross-schema foreign keys straightforward when needed

Design guidance:

- one migration stream for the whole database
- each migration may touch one or more schemas as needed
- prefer plain SQL migrations
- use cross-schema foreign keys when they improve integrity
- keep ownership clear even when schemas reference each other

Examples of acceptable cross-schema relationships:

- `memory.* -> agents(agent_key)`
- `agents.execution_accounts.account_address + environment -> hyperliquid activity journal` if a dedicated Hyperliquid accounts registry is introduced later
- `hyperliquid.execution_intents.agent_key -> agents(agent_key)`

## Runtime Process Model

The current leaning is one binary, not multiple deployables.

That one binary should still have clearly separated supervised internal tasks.

Possible runtime task layout:

1. `web`
2. `hyperliquid_sync`
3. `candle_scheduler`
4. `analysis_worker`
5. `daily_evaluator`

Responsibilities:

### `web`

- web UI
- internal API
- agent registry and runtime management
- read/write access to memory system
- operator controls

### `hyperliquid_sync`

- instrument sync (via NT)
- historical account-history sync via app-owned raw HTTP `/info` client
- ongoing HTTP polling for fills, funding, ledger, and historical orders
- websocket live fill feed deferred to a later step
- reconciliation / repair loops

### `candle_scheduler`

- detect new candle boundaries
- trigger `1m`, `15m`, `1h`, and `1d` jobs
- the `AgenticScheduler` is now the implementation of the
  OpenCode-side candle-aligned scheduler: it polls
  `agentic_job_schedules` every 10 seconds, claims due schedules
  (using `agentic::timeframe::next_due_after` to find the next UTC
  boundary for each schedule's `timeframe`), and dispatches them
  sequentially per agent while still allowing different agents to
  run concurrently. `job_key` is generated server-side as
  `"{job_kind}-{timeframe}"`, and the
  `(agent_key, job_kind, timeframe)` triple is the schedule's natural
  unique key.

### `analysis_worker`

- run analysis jobs
- read memory context
- write new memory records

### `daily_evaluator`

- compare expectations vs outcomes
- write evaluations and reflections

This separation should keep responsibilities clearer and reduce coupling.

The important distinction is:

- one deployable binary
- multiple supervised internal tasks or runtime modes

## Hyperliquid Integration

This is one of the most important architecture constraints.

Current observations:

- Hyperliquid docs explicitly point to an official Python SDK
- Hyperliquid docs do not clearly list an official Go SDK
- TypeScript SDKs appear to be community-maintained
- Rust support exists, but the official support story is less clear than Python

Rust options discussed so far:

- `hyperliquid-dex/hyperliquid-rust-sdk`
- `infinitefield/hypersdk`

Key concern:

- if the exchange integration is critical, Rust may require accepting more SDK risk than Python

This does not rule Rust out, but it should be treated as a real tradeoff.

Refined direction:

- prefer using the `hypersdk` Rust crate as the sole Hyperliquid integration surface
- prefer `hypersdk` clients over building a fresh exchange client from scratch
- avoid coupling the whole app to other runtime concepts unless that later proves necessary
- decision: use `hypersdk` for instruments, live account state, the live WebSocket, and signing/order submission; own a small raw HTTP `/info` client for the account-history endpoints it does not expose

## Practical Rust Tradeoffs

Reasons to choose Rust:

- existing familiarity
- explicit and maintainable system design
- strong fit for long-running services and workers
- one language across web, workers, and data processing

Reasons to hesitate:

- less batteries-included than Django
- no built-in admin
- more plumbing for internal tooling
- Hyperliquid integration is less clearly official

## Suggested Rust Variants

### Variant A: `actix-web` Based

Use when:

- the priority is a direct, pragmatic Rust web app
- handler ergonomics matter more than ecosystem purity
- existing Rust familiarity is stronger than framework preference

Stack:

- `actix-web`
- `askama`
- `sqlx`
- `tokio`
- `tracing`

### Variant B: `axum` Based

Use when:

- starting fresh and willing to choose the cleaner long-term service stack
- stronger `tower` ecosystem alignment is desirable
- the app may grow into a more composable internal platform

Stack:

- `axum`
- `askama`
- `sqlx`
- `tokio`
- `tracing`

## Current Leaning

If staying in Rust, the current likely options are:

1. `actix-web + askama + sqlx + tokio`
2. `axum + askama + sqlx + tokio`

`actix-web` is a valid option.
`axum` may be slightly cleaner if choosing from scratch.

The bigger architecture question is probably not `actix` vs `axum`.

The bigger question is whether the Hyperliquid integration risk is acceptable in a Rust-first architecture.

## Open Questions

1. Should V2 be entirely Rust, or should Hyperliquid integration live in a separate service later if needed?
2. Between `actix-web` and `axum`, which model feels more natural for the expected web UI and internal API?
3. Should the web app and workers be separate binaries, or one binary with different runtime modes?
4. How much internal admin tooling is needed early on?
5. ~~Which Rust Hyperliquid SDK is mature enough to trust for private trading actions?~~ Resolved: use `hypersdk` as the venue SDK (instruments, live state, WS, signing/order submission); own a thin raw HTTP `/info` client for the account-history endpoints it does not expose.

## Summary

Rust is still a serious option for V2.

The leading Rust architecture shape so far is:

- Rust backend
- Postgres
- `sqlx` migrations
- server-rendered templates
- separate worker processes
- careful evaluation of Rust Hyperliquid SDK choices

The main Rust framework options currently under consideration are:

- `actix-web`
- `axum`

This document should be expanded later as the V2 direction becomes clearer.
