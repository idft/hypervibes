---
slug: /development/architecture
---

# Architecture

HyperVibes is one Rust application process. It runs the Axum web server,
Postgres migrations, the Hyperliquid monitor, and the OpenCode scheduler in a
single Tokio runtime.

## Runtime

Startup performs the following work:

1. Loads configuration and connects to Postgres.
2. Runs the shared SQL migration stream.
3. Starts the Hyperliquid agent monitor.
4. Starts the OpenCode sub-agent scheduler.
5. Serves the web interface, static files, SSE streams, and the agent JSON API.

The main process owns graceful shutdown. The first `SIGINT` or `SIGTERM` stops
new scheduled work and lets in-flight agent dispatches drain. A second signal
ends that grace period. The API remains available during the normal drain so an
in-flight OpenCode MCP server can finish its API calls.

## Subsystems

| Subsystem | Responsibility |
| --- | --- |
| `agents` | Agent registry, API keys, OpenCode workspaces, selected instruments, and strategy prompts. User-owned encrypted Hyperliquid signing material lives with authentication records. |
| `harness` | Persisted unified sub-agents/runs, analysis-coding and provider-reload maintenance, recovery of orphaned runs, and OpenCode dispatch. |

## Harness sub-agents

`harness_sub_agents` is the single configuration record for every OpenCode sub-agent and
`harness_sub_agent_runs` is its durable execution queue. Candle sub-agents use the
`candle_closed` trigger, which the scheduler owns and advances at UTC candle
boundaries after the configured settling delay. The fixed event sub-agents use
`analysis_batch_completed` and `daily_review_completed`; they are dispatched
directly after their qualifying predecessor and deliberately have no event
outbox. `harness_maintenance_tasks` remains separate for workspace work,
analysis-coding promotion, and provider reloads.

Sub-agents can be deleted only while their runs and OpenCode sessions are idle.
Deletion first removes terminal sessions through the OpenCode API, then deletes
the sub-agent and cascades its runs and coding maintenance rows. Conversations remain
separate from harness sub-agents and runs.
| `opencode` | OpenCode HTTP client, session persistence access, and generated agent workspaces. |
| `memory` | Append-only, agent-owned analysis and review records plus links between records. |
| `hyperliquid` | Instrument reference data, account-history journal, live account state, signed order gateway, and order reconciliation. |
| `web` | Askama-rendered web interface, HTMX/SSE updates, static assets, and `/api/v1` agent endpoints. |
| `settings` | Global application settings, including the base OpenCode system prompt. |

Postgres is the durable system of record. The application uses the public
schema for registry and orchestration tables, and `memory`, `hyperliquid`, and
`opencode` schemas for their respective data. All migrations live in
`migrations/` and run at application startup.

## Agent Execution

A user first configures and approves one user-owned Hyperliquid trading
signer on the Account page. Agent creation then selects an exclusive main or
sub-account, creates an agent API key, creates default prompts and schedules,
and generates the agent workspace after funding is completed or skipped.

The scheduler polls every 10 seconds. It claims due work transactionally and
uses independent analysis and trading lanes per agent, while allowing work for
different agents to run concurrently. Built-in work includes:

- scheduled `analysis`, `trading`, and `daily_review` sub-agents
- an `analysis_batch_completed` hook that can dispatch `market_analysis`
- request-gated `analysis_coding` follow-up work after a daily review

For a dispatch, the OpenCode backend creates a session in the isolated run or
candidate workspace and invokes the appropriate OpenCode command. The initial prompt contains the
agent and sub-agent context, selected instruments, the sub-agent-specific strategy prompt,
the latest `agent_learnings` memory, the global prompt, and a live
account snapshot for trading work. That snapshot includes per-stream data
authority and monitor health; unavailable data is never represented as an
empty account. The order gateway rejects new agent exposure until the
clearinghouse and open-orders streams are current, while reduce-only orders
remain available for risk reduction.

Market-analysis is the authority for market thesis and execution conditions.
Trading executes its latest fresh handoff without market-data access or package
code; it bases order and position management on memories, account and order
state, and its approved HyperVibes MCP tools only.

Run state is persisted. On startup and periodically thereafter, the scheduler
resumes queued runs and recovers stale running runs so interrupted dispatches
do not block subsequent work.

### Isolated Workspace Foundation

Phase 2 persists a versioned run-context snapshot and run-artifact lifecycle
record before isolated dispatch is enabled. The snapshot carries only run input
and capability identity; validation rejects sensitive gateway and credential
fields, including normalized field-name variants and known runtime credential
forms. Schema version one is a closed, typed JSON contract; adding a nested
input or capability requires a reviewed schema version rather than an opaque
metadata field.
Artifact paths are never stored in Postgres. The workspace controller
derives them from the owned agent key and run ID as
`/workspaces/runs/<agent-key>/<run-id>/workspace`.

The controller can idempotently create, inspect, scrub the root runtime `.env`,
and delete run workspaces. Equivalent state and controller paths exist for an
internal conversation UUID at
`/workspaces/conversations/<agent-key>/<conversation-id>/workspace`; external
channel keys are not filesystem identities. The scheduler and conversation
service still use the active workspace during this foundation phase. Phase 3
and Phase 4 respectively adopt these isolated directories for dispatch and
conversation turns.

Notifications may record a system-supplied run or conversation provenance and
capability-schema binding. A scoped source must be owned by the agent and have
the relevant durable notification authority. Provenance never stores a gateway
token or destination, and deleting a source run or conversation preserves the
notification record.

Agent conversations are separate from scheduled sub-agents and `harness_sub_agent_runs`. Each
`agent_conversations` row maps one user or future-gateway conversation to
one OpenCode session. Chat transcript, tool activity, errors, and context
telemetry are mirrored from OpenCode and delivered to the browser as complete
HTMX SSE partial snapshots. Future channels use the same mapping with their
channel and external conversation key.

## Boundaries

OpenCode performs LLM and tool execution. HyperVibes retains authority over
agent identity, scheduling, memory, account state, and exchange execution.
The workspace MCP server uses the agent API key to call HyperVibes; it does
not receive the user's Hyperliquid private key. Exchange orders are signed by
the owner's trading signer and target the agent's stored trading account.

The agent API is authenticated with a Bearer API key. The resolved `agent_key`
is the data-ownership boundary for account, memory, and order operations.

OpenCode is the sole execution backend. Its HTTP endpoint is configured at the
application level with `OPENCODE_BASE_URL`, rather than being assigned through
database runtime rows.

## Coding Isolation

An agent's only durable filesystem state is its Coding package at
`packages/<agent-key>/` under the configured workspace root. It contains
`manifest.json` plus any coding-agent-defined files, and it is created by the
first successful coding promotion. Analysis coding runs in an isolated
candidate workspace under `coding/<agent-key>/<task-id>/workspace`; candidate
creation copies the package root into the candidate's `scripts/user/` tree.
The model receives path-scoped native OpenCode filesystem permissions that can
edit only approved files under candidate `scripts/user`.
Pyright supplies Python diagnostics for those native reads and edits. Runs and
conversations use their own isolated directories; coding holds a write lease
only while promoting a validated candidate and verifying that the promoted
tree has the same hash.

The manifest is a validation registry, not a runtime authorization list: the
analysis agent may inspect and directly execute any Python file below its
run-local `scripts/user/` copy of the package, and trading may not read or
execute package code.

Candidate validation runs through a fixed local MCP tool in the OpenCode
analysis runtime, not through a HyperVibes HTTP endpoint. The tool accepts no
executable or path arguments and records a task-scoped result bound to the
candidate tree hash. The validator compiles and scans the complete candidate
tree and runs the complete deterministic fixture suite for every declared
validation target, naming the target in each check. The worker recomputes the
hash after generation, so any write after validation fails promotion closed.

Promotion swaps directories, never individual files. A JSON journal records
each swap phase, and startup recovery restores the previous tree when a process
stops before a verified completion. The maintenance task and linked harness
run remain the durable lifecycle record in Postgres.

## Configuration

Required configuration:

- `AGENTS_ENCRYPTION_KEY`: 32-byte key encoded as 64 hexadecimal characters
- `AGENTS_ENCRYPTION_KEY_ID`: identifier stored alongside encrypted keys
- `WORKSPACE_CONTROL_API_KEY`: shared credential for the workspace controller
- database configuration through `DATABASE_URL`, or the `POSTGRES_*` variables

Operational configuration includes `APP_BIND_ADDR` or `APP_HOST` and
`APP_PORT`, `APP_CACHE_DIR`, workspace root settings, the API base URL visible
inside OpenCode workspaces, `OPENCODE_BASE_URL`, and OpenCode Basic Auth credentials. See
`.env.example` for the complete local-development configuration.

The application defaults to `127.0.0.1:3003`. The workspace-facing API URL is
configured separately with `HYPERVIBES_AGENT_API_BASE_URL`; it must resolve
from the OpenCode container or runtime.

## Frontend

The web interface is server-rendered with Askama. HTMX handles partial
updates and SSE publishes live account, memory, and database-notified agent run
changes. The shared Postgres notification listener fans `harness_sub_agent_runs` changes
out to both run-detail streams and the Sub-agents tab's Recent Runs section; session
notifications are used only by run-detail transcript and summary streams.
Frontend source is in `assets/`; `build.rs` builds the Tailwind and esbuild
output when application assets or templates change.
## Workspace Controller And Volumes

The HyperVibes service never mounts or accesses agent workspace files. The
OpenCode container owns the `agent_workspaces` named volume at `/workspaces` and
runs the `workspace-controller` HTTP process alongside OpenCode. The HyperVibes
service calls its private Compose-network `/v1` API with
`WORKSPACE_CONTROL_API_KEY` for workspace lifecycle operations and bounded,
read-only workspace-browser requests. This controller is not an agent MCP tool;
agents cannot create, delete, inspect, promote, or browse workspaces through it.

Set `WORKSPACE_CONTROL_API_KEY` for both the HyperVibes and OpenCode services.
In production, the controller is private to the Compose network at
`http://opencode:14097` and has no host port. The development stack publishes
it only at `127.0.0.1:${WORKSPACE_CONTROL_PORT:-14097}`. Do not expose it
publicly or place the key in an agent workspace, prompt, MCP configuration, or
agent `.env`.

Back up the `agent_workspaces` volume with Podman volume tooling. Preserve the
`opencode_data` volume independently because it contains global provider
credentials and OAuth state. Back up the production `podman-compose.yaml` with
the database because it contains the agent-encryption key.

A rollout uses a fresh `agent_workspaces` volume. Back up the database, retain
the legacy host `workspaces/` directory outside normal operation, deploy, and
then recreate required agents. The application does not copy or remove legacy
workspace data.
