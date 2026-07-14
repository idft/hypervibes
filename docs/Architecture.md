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
| `agents` | Agent registry, encrypted Hyperliquid signing keys, API keys, runtime assignment, selected instruments, and strategy prompts. |
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

An operator creates an agent by selecting an enabled OpenCode runtime and
providing a Hyperliquid private key. The application derives the wallet
address, encrypts the key before storing it, creates an agent API key, creates
default prompts and schedules, and generates the agent workspace.

The scheduler polls every 10 seconds. It claims due work transactionally and
uses independent analysis and trading lanes per agent, while allowing work for
different agents to run concurrently. Built-in work includes:

- scheduled `analysis`, `trading`, and `daily_review` jobs
- an `analysis_batch_completed` hook that can dispatch `market_analysis`
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
not receive the agent's Hyperliquid private key.

The agent API is authenticated with a Bearer API key. The resolved `agent_key`
is the data-ownership boundary for account, memory, and order operations.

Only the `opencode` backend is currently supported. Backend and runtime fields
remain in the registry schema to leave room for a future backend without
changing agent ownership or orchestration concepts.

## Configuration

Required configuration:

- `AGENTS_ENCRYPTION_KEY`: 32-byte key encoded as 64 hexadecimal characters
- `AGENTS_ENCRYPTION_KEY_ID`: identifier stored alongside encrypted keys
- database configuration through `DATABASE_URL`, or the `POSTGRES_*` variables

Operational configuration includes `APP_BIND_ADDR` or `APP_HOST` and
`APP_PORT`, `APP_CACHE_DIR`, workspace root settings, the API base URL visible
inside OpenCode workspaces, and OpenCode Basic Auth credentials. See
`.env.example` for the complete local-development configuration.

The application defaults to `127.0.0.1:3000`. The workspace-facing API URL is
configured separately with `VIBETRADING_AGENT_API_BASE_URL`; it must resolve
from the OpenCode container or runtime.

## Frontend

The operator interface is server-rendered with Askama. HTMX handles partial
updates and SSE publishes live account and memory changes. Frontend source is
in `assets/`; `build.rs` builds the Tailwind and esbuild output when application
assets or templates change.
