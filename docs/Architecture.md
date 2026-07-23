# Architecture

Vibetrading is one Rust application process. It runs the Axum web server,
Postgres migrations, the Hyperliquid monitor, and the OpenCode scheduler in a
single Tokio runtime.

## Runtime

Startup performs the following work:

1. Loads configuration and connects to Postgres.
2. Runs the shared SQL migration stream.
3. Starts the Hyperliquid agent monitor.
4. Starts the OpenCode job scheduler.
5. Serves the operator UI, static files, SSE streams, and the agent JSON API.

The main process owns graceful shutdown. The first `SIGINT` or `SIGTERM` stops
new scheduled work and lets in-flight agent dispatches drain. A second signal
ends that grace period. The API remains available during the normal drain so an
in-flight OpenCode MCP server can finish its API calls.

## Subsystems

| Subsystem | Responsibility |
| --- | --- |
| `agents` | Agent registry, API keys, OpenCode workspaces, selected instruments, and strategy prompts. User-owned encrypted Hyperliquid signing material lives with authentication records. |
| `agentic` | Persisted schedules, hooks, runs, workspace maintenance, recovery of orphaned runs, and OpenCode dispatch. |
| `opencode` | OpenCode HTTP client, session persistence access, and generated agent workspaces. |
| `memory` | Append-only, agent-owned analysis and review records plus links between records. |
| `hyperliquid` | Instrument reference data, account-history journal, live account state, signed order gateway, and order reconciliation. |
| `web` | Askama-rendered operator UI, HTMX/SSE updates, static assets, and `/api/v1` agent endpoints. |
| `settings` | Global application settings, including the base OpenCode system prompt. |

Postgres is the durable system of record. The application uses the public
schema for registry and orchestration tables, and `memory`, `hyperliquid`, and
`opencode` schemas for their respective data. All migrations live in
`migrations/` and run at application startup.

## Agent Execution

An operator first configures and approves one user-owned Hyperliquid trading
signer on the Account page. Agent creation then selects an exclusive main or
sub-account, creates an agent API key, creates default prompts and schedules,
and generates the agent workspace after funding is completed or skipped.

The scheduler polls every 10 seconds. It claims due work transactionally and
uses independent analysis and trading lanes per agent, while allowing work for
different agents to run concurrently. Built-in work includes:

- scheduled `analysis`, `trading`, and `daily_review` jobs
- an `analysis_batch_completed` hook that can dispatch `market_analysis`
- request-gated `analysis_coding` follow-up work after a daily review
- queued workspace regeneration or hard reset

For a dispatch, the OpenCode backend creates a session in the agent workspace
and invokes the appropriate OpenCode command. The initial prompt contains the
agent and job context, selected instruments, the job-specific strategy prompt,
the latest `agent_learnings` memory, the global operator prompt, and a live
account snapshot for trading work.

Run state is persisted. On startup and periodically thereafter, the scheduler
recovers stale queued or running runs so interrupted dispatches do not block an
agent lane indefinitely.

## Boundaries

OpenCode performs LLM and tool execution. Vibetrading retains authority over
agent identity, scheduling, memory, account state, and exchange execution.
The workspace MCP server uses the agent API key to call Vibetrading; it does
not receive the user's Hyperliquid private key. Exchange orders are signed by
the owner's trading signer and target the agent's stored trading account.

The agent API is authenticated with a Bearer API key. The resolved `agent_key`
is the data-ownership boundary for account, memory, and order operations.

OpenCode is the sole execution backend. Its HTTP endpoint is configured at the
application level with `OPENCODE_BASE_URL`, rather than being assigned through
database runtime rows.

## Coding Isolation

Analysis coding runs in a candidate workspace under the configured
workspace root. The model receives path-scoped native OpenCode filesystem
permissions that can edit only approved files under candidate `scripts/user`.
Pyright supplies Python diagnostics for those native reads and edits. Normal analysis, trading, and review jobs hold read
leases on the live workspace; coding holds a write lease only while
promoting a validated candidate and verifying that the promoted tree has the
same hash.

Candidate validation runs through a fixed local MCP tool in the OpenCode
analysis runtime, not through a Vibetrading HTTP endpoint. The tool accepts no
executable or path arguments and records a task-scoped result bound to the
candidate tree hash. The worker recomputes that hash after generation, so any
write after validation fails promotion closed.

Promotion swaps directories, never individual files. A JSON journal records
each swap phase, and startup recovery restores the previous tree when a process
stops before a verified completion. The maintenance task and linked agentic
run remain the durable lifecycle record in Postgres.

## Configuration

Required configuration:

- `AGENTS_ENCRYPTION_KEY`: 32-byte key encoded as 64 hexadecimal characters
- `AGENTS_ENCRYPTION_KEY_ID`: identifier stored alongside encrypted keys
- database configuration through `DATABASE_URL`, or the `POSTGRES_*` variables

Operational configuration includes `APP_BIND_ADDR` or `APP_HOST` and
`APP_PORT`, `APP_CACHE_DIR`, workspace root settings, the API base URL visible
inside OpenCode workspaces, `OPENCODE_BASE_URL`, and OpenCode Basic Auth credentials. See
`.env.example` for the complete local-development configuration.

The application defaults to `127.0.0.1:3000`. The workspace-facing API URL is
configured separately with `VIBETRADING_AGENT_API_BASE_URL`; it must resolve
from the OpenCode container or runtime.

## Frontend

The operator interface is server-rendered with Askama. HTMX handles partial
updates and SSE publishes live account, memory, and database-notified OpenCode
run-detail changes. Frontend source is in `assets/`; `build.rs` builds the
Tailwind and esbuild output when application assets or templates change.
