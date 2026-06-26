# OpenCode Backend Plan

Related docs:

- `Backends.md` for the multi-backend execution model
- `Agents.md` for the agent registry and account ownership model
- `Hermes.md` for the existing Hermes backend and profile distribution model
- `Memory.md` for analysis memory ownership and handoff semantics
- `Hyperliquid.md` for execution gateway and account sync ownership

## Goal

Add OpenCode as a second agent execution backend alongside Hermes.

OpenCode is not a Hermes replacement. It is the backend-owned, app-scheduled
runtime option for agents that should be dispatched and observed by the
Vibetrading backend.

The intended ownership model is:

- Vibetrading owns agent identity, scheduling for OpenCode agents, run state,
  memory, account state, and order execution.
- OpenCode owns LLM/tool execution inside a containerized runtime.
- OpenCode sessions are invoked programmatically by the Vibetrading backend.
- Hermes remains available for agents that should keep using Hermes profiles,
  Hermes cron, persistent memory, and Hermes self-evaluation behavior.
- Hyperliquid private keys remain backend-side and must not be exposed to
  OpenCode.

This is a major architecture addition and should be implemented in phases.

Current implemented slice:

- `agent_runtimes` exists as the first generic backend runtime table
- migrations seed one OpenCode runtime, `opencode-local`, pointing at
  `http://localhost:14096`
- agents can now be linked to either Hermes or OpenCode runtimes in the UI
- creating an OpenCode agent now generates a per-agent workspace under
  `workspaces/agents/<agent_key>` on the host and
  `/workspaces/agents/<agent_key>` in the container
- workspace metadata is stored in `agents.runtime_config` as:
  - `workspace_host_path`
  - `workspace_container_path`
  - `profile_source`
- generated workspaces include a non-secret `generated/agent.json` file, an
  agent-scoped `.env` file (consumed by the `vibetrading` MCP server inside
  the container), and a project-level `opencode.json` registering the local
  `vibetrading` MCP server
- the custom OpenCode container image installs the `vibetrading` MCP server at
  `/opt/vibetrading/mcp/` and includes its Python dependencies
- OpenCode agents interact with the Vibetrading backend exclusively through
  the `vibetrading` MCP server; there is no workspace-local Python API
  client anymore
- scheduling, session creation, and run tracking are not implemented yet

The rest of this document still captures the larger OpenCode backend plan.

## High-Level Architecture

The target OpenCode path is:

```text
Vibetrading backend
  -> AgenticScheduler for OpenCode agents only
  -> OpenCodeBackend adapter
  -> OpenCode HTTP API / Rust SDK
  -> OpenCode server container
  -> per-agent OpenCode project workspace
  -> Vibetrading APIs/tools for memory, account state, and execution
```

Runtime decisions so far:

- run one OpenCode server container for all initial OpenCode agents
- use one normal shared filesystem workspace volume
- use one OpenCode project directory per Vibetrading agent
- create a fresh OpenCode session for every scheduled run
- store Vibetrading run lifecycle in Postgres
- use the OpenCode database plugin to mirror transcripts, tool calls, token
  usage, and costs into Postgres
- build a custom OpenCode image based on the upstream image with Python, `uv`,
  the database plugin, and Vibetrading runtime dependencies

OpenCode is not expected to be embedded in the Rust backend. OpenCode is a
TypeScript/Bun application packaged as a standalone executable, so the practical
integration is HTTP from Rust into a separately running server.

## Backend Adapter Role

OpenCode is represented in Vibetrading by the generic backend model described in
`Backends.md`.

At a high level:

- each OpenCode container or deployment is one `agent_runtime` row with
  `backend_kind = 'opencode'`
- each OpenCode agent has `agents.runtime_id` pointing at that runtime row
- `agents.backend_kind` is denormalized as `opencode` for fast filtering and UI
  badges
- OpenCode-specific per-agent settings live in `agents.runtime_config`
- shared agent identity, memory, account state, and execution APIs remain
  runtime-agnostic

The `OpenCodeBackend` adapter is active: the `AgenticScheduler` walks OpenCode
job schedules and dispatches due runs through this adapter.

The adapter owns OpenCode-specific resolution such as:

- which OpenCode project directory belongs to an agent
- which command and OpenCode agent should be used for each `job_kind`
- how to create a session
- how to attach the session to the correct directory
- how to record the underlying session reference in `agentic_runs.backend_run_ref`
- how to report runtime metadata and health

The adapter should hide these details from the generic agent registry schema.

## Runtime Profile Source

Do not put trading-agent OpenCode commands, agents, or skills in the repository
root `.opencode/` directory. That directory is for local developer OpenCode
configuration.

The OpenCode runtime assets should live in a separate distribution-like source
directory, similar in purpose to the current Hermes profile distribution.

Source layout:

```text
agent-runtime/opencode/
├── README.md
├── SOUL.md
├── opencode.json.template
├── AGENTS.md.template
├── commands/
│   ├── vibetrading-analysis.md
│   └── vibetrading-trading.md
├── agents/
│   ├── analysis.md
│   └── trading.md
├── skills/
├── scripts/
├── mcp/
│   ├── README.md
│   ├── requirements.txt
│   ├── server.py
│   └── test_server.py
└── container/
    └── opencode.jsonc
```

The MCP server source under `mcp/` is the single source of truth for the
Vibetrading MCP server. It is baked into the custom OpenCode image at
`/opt/vibetrading/mcp/` by `containers/opencode/Dockerfile`. Generated
workspaces no longer contain a Python API client; the `vibetrading` MCP
server is the only Vibetrading API integration path.

Generated agent workspaces may contain project-local `.opencode/` directories if
OpenCode requires that layout for command, agent, and skill discovery. The source
of truth should still be `agent-runtime/opencode/`, not the repository root
`.opencode/`.

## Agent Workspaces

Each OpenCode-backed Vibetrading agent should get its own OpenCode project
directory under a shared workspace volume.

Generated layout:

```text
/workspaces/agents/<agent_key>/
├── opencode.json
├── AGENTS.md
├── .opencode/
│   ├── commands/
│   ├── agents/
│   └── skills/
├── generated/
│   └── agent.json
├── scripts/
│   ├── generated/
│   └── user/
├── data/
└── scratch/
```

The generated `opencode.json` registers the `vibetrading` MCP server as a
local stdio process. Each workspace runs its own MCP process; the process
reads that workspace's `.env` to authenticate against the Vibetrading
backend with the agent's bearer token.

Ownership rules:

- Vibetrading may create and update generated files.
- OpenCode agents may write only to explicitly writable paths.
- The analysis agent may write Python scripts under `scripts/user/`. Those
  scripts are for analysis computation only and must not be used to call
  Vibetrading APIs.
- The trading agent should not write Python scripts in the initial design.
- The backend must preserve agent-authored paths when regenerating runtime
  files.

The backend writes generated workspaces to `workspaces`, and the OpenCode
container sees the same bind mount at `/workspaces`.

The OpenCode container also bind-mounts
`agent-runtime/opencode/container/opencode.jsonc` to
`/opencode-data/opencode.jsonc`. Shared plugins, including the dotenv plugin,
belong in that global container config rather than in generated workspace
`opencode.json` files.

Agent workspaces should be backed up. They are not intended to be disposable
because analysis agents may write durable Python analysis scripts.

Git should not be initialized in agent workspaces as part of the initial plan.

## Scheduler Model

OpenCode scheduling is owned by Vibetrading, unlike Hermes scheduling.

The existing `AgentOrchestrator` is better understood as a Hyperliquid account
sync and monitoring task. The OpenCode backend should introduce a separate
agentic scheduler rather than mixing LLM run scheduling into the Hyperliquid
sync loop.

Proposed background systems:

```text
HyperliquidAgentMonitor
  per enabled agent:
    startup account sync
    historical orders sync
    live WebSocket state
    order reconciliation

AgenticScheduler
  per enabled OpenCode agent schedule:
    due-time calculation
    OpenCodeBackend dispatch
    run status tracking
    timeout/failure handling
```

The scheduler must only dispatch agents whose `backend_kind` is `opencode`.
Hermes agents continue to be scheduled by the operator's Hermes cron and are
observed through `/api/v1/job-context` check-ins.

The current orchestrator may be renamed or moved later to better reflect its
Hyperliquid-specific responsibilities.

## DB-Backed Job Schedules

OpenCode jobs should be DB-backed and configurable per agent. Do not hardcode
only one analysis loop and one trading loop.

Examples:

```text
btc-momentum:
  trading-1m
  analysis-15m
  analysis-1h
  analysis-1d
```

Schedules should be independently enabled and disabled, separate from the parent
agent's enabled flag. This allows analysis-only mode, pausing trading while
research continues, and disabling an expensive or broken job without disabling
the whole agent.

The job kind should derive the OpenCode command and OpenCode agent. These should
not be schedule-configurable in the initial design.

Initial mapping inside the OpenCode adapter:

```text
analysis -> command vibetrading-analysis, agent analysis
trading  -> command vibetrading-trading,  agent trading
```

Schedule rows should store cadence, model choice, timeout, and an optional
operator prompt. The operator prompt is not the full executable prompt. It is a
small per-job instruction fragment injected into the canonical OpenCode command
or skill flow.

Proposed table shape:

```text
agentic_job_schedules
  id
  agent_key
  job_key
  job_kind
  enabled
  interval_seconds
  next_run_at
  model_provider_id
  model_id
  timeout_seconds
  operator_prompt
  created_at
  updated_at
```

This table is OpenCode-only at first. It does not need backend-specific columns.

Recommended initial defaults:

```text
trading-1m:
  job_kind: trading
  interval: 60 seconds
  timeout: 45 seconds

analysis-15m:
  job_kind: analysis
  interval: 900 seconds
  timeout: 600 seconds
```

Longer timeframe analysis jobs such as `analysis-1h` and `analysis-1d` can be
added per agent as configuration.

## Run Tracking

Vibetrading should store canonical OpenCode run lifecycle state in its own
tables. Full transcripts, tool calls, token usage, and cost data should come from
the OpenCode database plugin rather than being duplicated in Vibetrading run
rows.

Proposed statuses:

```text
queued
running
succeeded
failed
aborted
skipped
```

Proposed run table shape:

```text
agentic_runs
  id
  schedule_id
  agent_key
  job_key
  job_kind
  status
  backend_run_ref
  model_provider_id
  model_id
  scheduled_for
  started_at
  finished_at
  timeout_seconds
  error_summary
  created_at
  updated_at
```

`backend_run_ref` stores the underlying OpenCode session reference or equivalent
adapter-level identifier. The table intentionally avoids OpenCode-specific
columns such as command, agent, project path, or session ID.

The `job_kind -> command/agent` mapping lives inside the OpenCode adapter. If
historical debugging later needs a resolved command snapshot, add it only after
there is a concrete need.

Do not copy large run context blobs into `agentic_runs` by default. Link runs to
OpenCode plugin data with `backend_run_ref`.

Skipped runs should be inserted, not merely logged. A skipped run represents a
due schedule that did not start because another run for the same schedule was
already active.

This table is OpenCode-only at first. Whether Hermes activity should ever be
folded into `agentic_runs` is deferred.

## Scheduling Semantics

For each enabled OpenCode schedule whose parent agent is enabled:

```text
if now >= next_run_at:
  if active run exists for the same schedule:
    insert skipped run
    advance next_run_at
  else:
    insert queued run
    advance next_run_at
    dispatch through OpenCodeBackend
```

Initial concurrency rule:

```text
max one active run per schedule
```

If a scheduled run is still active when the next interval arrives, skip the new
run. Do not queue overlapping work.

If OpenCode is unavailable or a run fails, mark the run failed. Do not
automatically disable the agent or schedule in the initial design.

Disabling an agent or schedule prevents new runs. It does not need to abort an
already in-progress OpenCode session in the initial design.

## Analysis And Trading Behavior

OpenCode should be introduced conservatively.

Recommended rollout order:

1. Analysis jobs that can write memories.
2. Trading jobs in dry-run or proposal mode.
3. Real trading tools only after the OpenCode runtime has been observed in use.

Analysis jobs:

- may use Python
- may write and reuse Python analysis scripts under the agent workspace
- may write memories through Vibetrading APIs/tools
- must not place or cancel orders

Trading jobs:

- consume current account state and relevant analysis memories
- place or cancel orders only through Vibetrading execution APIs/tools
- should not directly sign Hyperliquid orders
- should not write Python scripts in the initial design

The exact OpenCode skills, commands, and permissions are deferred to the runtime
implementation phase.

## Shared API Surface

OpenCode agents should use the same Vibetrading API surface as Hermes agents.

Shared invariants:

- agents authenticate with agent-scoped API keys
- agents only read and write their own memory
- agents get account state through Vibetrading
- agents place and cancel orders only through Vibetrading
- Hyperliquid private keys never enter Hermes or OpenCode
- runtime-specific details stay behind adapters or `agents.runtime_config`

OpenCode should not receive privileged direct access to memory, account state, or
execution paths that Hermes does not have.

## OpenCode Database Plugin

Use the OpenCode database plugin to mirror OpenCode runtime detail into
Postgres, including:

- sessions
- messages
- message parts
- tool executions
- token usage
- cost estimates
- session errors

### Chosen Package

- Package: `opencode-database-plugin@1.0.12` (pinned in
  `agent-runtime/opencode/container/opencode.jsonc`)
- Upstream repository: https://github.com/aemr3/opencode-database-plugin
- License: Apache-2.0
- Vendored SQL: `migrations/vendor/opencode-database-plugin/schema.sql`
  (Apache-2.0 license kept alongside as `LICENSE`)

The database plugin is registered alongside the shared dotenv plugin in
the container-global `opencode.jsonc` so generated per-agent workspaces
do not need to declare it.

### Schema And Configuration

All plugin-owned objects live under a dedicated `opencode` Postgres
schema. The plugin's SQL is unqualified, so the Postgres connection used
by OpenCode must set `search_path=opencode,public`. This is configured
once in `podman-compose.yaml` via the `OPENCODE_DATABASE_URL` query
parameter:

```yaml
OPENCODE_DATABASE_URL: postgres://${POSTGRES_USER:-vibetrading}:${POSTGRES_PASSWORD:-vibetrading}@postgres:5432/${POSTGRES_DB:-vibetrading}?search_path=opencode,public
OPENCODE_DB_QUERY_TIMEOUT: ${OPENCODE_DB_QUERY_TIMEOUT:-10000}
```

The compose hostname is `postgres` so the OpenCode process reaches the
dev Postgres service inside the compose network; do not switch this to
`localhost`. `OPENCODE_DB_QUERY_TIMEOUT` is exposed for operators in
`.env.example` and defaults to `10000` ms.

The `opencode` schema and all plugin tables, indexes, triggers, the
`update_updated_at_column()` function, and the `conversation_view` view
are created by migration `0007_opencode_database_plugin.sql`. The
upstream DDL is vendored at
`migrations/vendor/opencode-database-plugin/schema.sql` for reproducible
migrations and license-tracking.

### Plugin Tables

The plugin creates these tables under the `opencode` schema:

- `sessions`
- `messages`
- `message_parts`
- `tool_executions`
- `session_errors`
- `commands`
- `compactions`

It also creates:

- `conversation_view` (view)
- `update_updated_at_column()` (function used by the per-table
  `updated_at` triggers)

### Failure Handling

Plugin logging failure should not stop a trading or analysis run, but the
system should surface degraded observability. Wrap plugin write calls in
a `tracing` span and log a warning (not an error) when a write fails so
the agent loop is never blocked by the database plugin.

## Configuration And Deployment

OpenCode runs in a custom container image built from
`containers/opencode/Dockerfile`. The base image is the upstream
`ghcr.io/anomalyco/opencode:1.17.11` (pinned to match the version the
compose file used to consume directly).

The custom image installs:

- the `uv` binary, copied from `docker.io/astral/uv:0.10-alpine`
- Python 3 (the upstream image is Alpine-based and ships neither Python
  nor pip)
- the Vibetrading MCP server source at `/opt/vibetrading/mcp/`
- a Python virtualenv at `/opt/vibetrading/mcp/.venv/`, created with
  `uv venv`, with the MCP server dependencies from
  `agent-runtime/opencode/mcp/requirements.txt` installed via `uv pip`

The `vibetrading` MCP server is launched per agent workspace via the
project-level `opencode.json` that the backend writes for each generated
workspace. The launch command resolves to the venv's Python interpreter
so the image does not depend on `PATH` or shell startup files.

Runtime volumes:

```text
/opencode-data
/workspaces
```

Provider credentials should be passed to the OpenCode container with environment
variables when possible.

OpenCode server auth should be enabled, even on the internal container network.

Agent-scoped Vibetrading API credentials are isolated per workspace. Each
MCP server process reads only its own workspace `.env`, and the bearer
token never leaves that process. There is no shared, globally authenticated
MCP daemon.

## Operator UX

OpenCode should appear as one available backend kind, not as the only runtime.

Operator-facing behavior:

- operators create or register an OpenCode runtime instance separately from
  agents
- creating an agent requires explicitly selecting a runtime; there is no silent
  default
- agents show a backend badge such as `Hermes` or `OpenCode`
- OpenCode-backed agents show schedule and run history UI
- Hermes-backed agents keep Hermes setup/check-in guidance
- there is no runtime switcher in the initial design

The UI should make scheduling ownership clear: OpenCode schedules are managed by
Vibetrading, while Hermes schedules remain managed by Hermes cron.

## Schema Direction

The shared agent-side schema belongs in `Backends.md`, not this document.

This document only owns OpenCode-side tables and integration concerns:

- `agentic_job_schedules`
- `agentic_runs`
- `opencode` plugin schema
- generated workspaces
- OpenCode adapter behavior

Agent-side changes such as `agent_runtime`, `agents.runtime_id`,
`agents.backend_kind`, and `agents.runtime_config` are part of the generic
backend model.

## Migration Direction

Existing Hermes agent state does not need to be migrated into the OpenCode
system. The OpenCode runtime can start fresh, and Hermes agents can keep running
unchanged.

Suggested implementation phases:

1. Document the generic backend model in `Backends.md`.
2. Spike OpenCode server directory-scoped project behavior.
3. Add OpenCode container configuration for local development.
4. Add `agent_runtime` and generic agent runtime fields when schema work begins.
5. Add `agentic_job_schedules` and `agentic_runs` for OpenCode agents.
6. Vendor the OpenCode database plugin schema under the `opencode` schema.
7. Add the OpenCode runtime profile source directory.
8. Add per-agent workspace generation.
9. Add a scheduler with a mock OpenCode backend.
10. Wire the scheduler to OpenCode session and command execution.
11. Enable analysis jobs first.
12. Enable trading jobs in dry-run/proposal mode.
13. Enable real trading tools last.
14. Add UI work for backend badges, per-backend setup guidance, mandatory
    runtime picker on create, OpenCode schedules, and OpenCode run history.

## Implementation TODO

This section expands the implementation phases above into a sequenced backlog
of small, reviewable chunks. Each chunk should be small enough to ship behind
a single review and is ordered to respect dependencies between doc, schema,
and runtime work. Items in earlier phases unblock items in later phases but
should not block each other within a phase where possible.

### Phase 0: Documentation Foundation

- **0.1 - Add `docs/Backends.md` with the generic backend model.** Create the
  doc that this plan already references. Define the `agent_runtime` row
  shape, the meaning of `agents.runtime_id`, `agents.backend_kind`, and
  `agents.runtime_config`, and the shared invariants. This precedes any
  schema work.
- **0.2 - Update `docs/README.md` doc index.** Add `Backends.md` to the docs
  list. Ship alongside 0.1.
- **0.3 - Add this Implementation TODO section to `OpenCode.md`.** Capture
  the concrete chunked plan inline. (This step.)

### Phase 1: Spikes and Investigation

These items are inputs to every later phase. They are intentionally small and
exist to remove unknowns from the plan's "Deferred Investigation Items" list.

- **1.1 - Spike the OpenCode HTTP API and directory-scoped sessions.** ✅ Done.
  Spiked against the local `opencode-local` runtime (OpenCode `1.17.11`) using
  the `/workspaces/agents/btc-2` workspace. Key finding: there is **no
  `x-opencode-directory` header**. The directory is a **`?directory=` query
  parameter** on essentially every project-scoped route, and it is **permanently
  bound to the session** at creation time. Follow-up `POST /session/{id}/message`
  and `POST /session/{id}/command` calls do not need to repeat it and cannot
  override it. See the "OpenCode HTTP API Notes" section below for the full
  verified request/response shape.
- **1.2 - Investigate the OpenCode database plugin schema.** ✅ Done as
  part of the OpenCode Database Plugin Foundation plan. The plugin's
  DDL was vendored at `migrations/vendor/opencode-database-plugin/`,
  and migration `0007_opencode_database_plugin.sql` applies it under
  the `opencode` schema by setting `search_path = opencode, public` in
  the migration. No patching of the upstream SQL is required. See
  "OpenCode Database Plugin" above for the resolved decision.
- **1.3 - Investigate permission handling for headless runs.** Verify how
  OpenCode behaves when a permission resolves to `ask`, and document the
  production permission configuration that prevents runs from blocking on
  interactive approval.
- **1.4 - Investigate per-agent Vibetrading API credential scoping for
  OpenCode.** Determine the cleanest way to give each OpenCode session
  access to its own agent's API key without making every agent's key
  globally available inside the container. Update `Backends.md` with the
  chosen shape.
- **1.5 - Promote resolved items out of "Deferred Investigation Items".** As
  1.1 through 1.4 are answered, move the answers into concrete decisions in
  `OpenCode.md` and remove the resolved bullets from the deferred list.

### Phase 2: Container and Local Dev

- **2.1 - Add the `opencode` service to `podman-compose.yaml`.** Mount
  `/workspaces` and `/opencode-data` volumes, expose the OpenCode HTTP port
  on the internal network only, and read provider credentials from env. Do
  not auto-start.
- **2.2 - Add a custom OpenCode Dockerfile at `containers/opencode/Dockerfile`.**
  Build on the upstream OpenCode image. Install the Vibetrading MCP server
  source under `/opt/vibetrading/mcp/` and a Python virtualenv with its
  dependencies. The database plugin and the in-workspace Python client are
  no longer in scope for this container.
- **2.3 - Pin a shared workspace path.** Ensure the OpenCode container and
  the backend container see `/workspaces` as the same absolute path so
  generated files work without translation.
- **2.4 - Configure OpenCode server auth.** Generate a server password, add
  it to `.env.example`, and inject it into the container env. Verify that
  anonymous calls to the API are rejected.

### Phase 3: Generic Backend Schema

- **3.1 - Migration: create the `agent_runtime` table.** Add a new migration
  with one row per backend deployment. Columns: `id`, `backend_kind`,
  `display_name`, `base_url`, `auth_secret_ref`, `metadata`, timestamps.
  CHECK constraint for `backend_kind IN ('hermes', 'opencode')`.
- **3.2 - Migration: add runtime columns to `agents`.** Add nullable
  `runtime_id`, `backend_kind`, and JSONB `runtime_config` to the existing
  `agents` table. Add a CHECK constraint for `backend_kind`.
- **3.3 - Backfill: register one Hermes runtime and assign existing agents
  to it.** Insert a single `agent_runtime` row representing the current
  Hermes install and update every existing `agents` row to point at it with
  `backend_kind = 'hermes'`.
- **3.4 - Tests for runtime resolution helpers.** Add unit tests for
  "resolve agent by api_key and load its runtime" and for the default
  Hermes assignment.

### Phase 4: Job Schedules and Runs Schema

- **4.1 - Migration: create the `agentic_job_schedules` table.** Add the
  schedule table per the plan. Include a CHECK constraint for `job_kind` and
  a unique index on `(agent_key, job_key)`.
- **4.2 - Migration: create the `agentic_runs` table.** Add the runs table
  per the plan with a `status` CHECK constraint, a `backend_run_ref` text
  column, and indexes on `(agent_key, job_key, started_at DESC)` and
  `(status)`.
- **4.3 - Add a small `agentic_runs` and `agentic_job_schedules` query
  module.** Provide `insert_queued`, `mark_running`, `mark_succeeded`,
  `mark_failed`, `insert_skipped`, and `find_active_for_schedule` helpers.
  Keep this small and query-focused.

### Phase 5: OpenCode Database Plugin Vendoring

- **5.1 - Vendor the OpenCode database plugin DDL.** ✅ Superseded by the
  OpenCode Database Plugin Foundation plan. The upstream DDL is vendored
  unchanged at `migrations/vendor/opencode-database-plugin/schema.sql`
  (commit `53fea73`, package `opencode-database-plugin@1.0.12`) alongside
  the upstream Apache-2.0 license.
- **5.2 - Qualify or patch the vendored DDL.** ✅ Superseded by the OpenCode
  Database Plugin Foundation plan. No patching of the upstream SQL was
  needed. Setting `search_path = opencode, public` in
  `0007_opencode_database_plugin.sql` causes the plugin's unqualified
  `CREATE TABLE` / `CREATE INDEX` / trigger / function / view statements
  to be created under the `opencode` schema without modification.
- **5.3 - Migration: create the `opencode` schema and apply the plugin DDL.**
  ✅ Done as `migrations/0007_opencode_database_plugin.sql` with a
  dedicated-schema downgrade at
  `migrations/0007_opencode_database_plugin.down.sql`.
- **5.4 - Surface plugin logging failures as degraded observability.** Wrap
  plugin calls in a `tracing` span and log a warning (not an error) when
  the plugin write fails, so analysis and trading runs are not blocked.

### Phase 6: Runtime Profile Source

- **6.1 - Create the `agent-runtime/opencode/` directory skeleton.** Add the
  empty directory layout per the plan with `.gitkeep` placeholders. No
  content yet.
- **6.2 - Add `agent-runtime/opencode/README.md`.** Explain the purpose of
  this directory, how it relates to the repo-root `.opencode/`, and how it
  gets baked into the custom OpenCode image.
- **6.3 - Add `agent-runtime/opencode/SOUL.md` and `AGENTS.md.template`.**
  Add the trading-agent OpenCode command context and a per-agent template
  that gets rendered into each workspace.
- **6.4 - Add command stubs.** Create `commands/vibetrading-analysis.md` and
  `commands/vibetrading-trading.md` with placeholders describing what each
  will do.
- **6.5 - Add agent stubs.** Create `agents/analysis.md` and
  `agents/trading.md` with placeholders.
- **6.6 - ~~Add the `vibetrading/py/` Python client skeleton.~~** Removed by
  the MCP runtime plan. The Python client has been replaced by the
  `vibetrading` MCP server under `agent-runtime/opencode/mcp/`. Generated
  workspaces no longer contain a workspace-local Python client.
- **6.7 - ~~Add `requirements.txt` for the workspace Python environment.~~**
  Removed by the MCP runtime plan. The MCP server has its own
  `mcp/requirements.txt`, installed at image build time into a fixed
  virtualenv at `/opt/vibetrading/mcp/.venv/`.

### Phase 7: Workspace Generation

- **7.1 - Add a `workspaces_root` config value.** Add a new field to
  `AppConfig` for the absolute path where per-agent OpenCode workspaces
  live. Default to a sensible local-dev path.
- **7.2 - Add `src/agent_runtime/opencode/workspace.rs` with directory
  creation.** Create the per-agent workspace tree under
  `/workspaces/agents/<agent_key>/` with the layout from the plan.
  Idempotent: do not error if the tree already exists.
- **7.3 - Render `opencode.json` and `AGENTS.md` from the profile source.**
  Copy `opencode.json.template` and `AGENTS.md.template` into the
  workspace, substituting `agent_key` and any other per-agent fields.
  Track rendered values in `generated/agent.json`.
- **7.4 - Place `.opencode/{commands,agents,skills}` from the profile
  source.** Copy these subtrees from `agent-runtime/opencode/` into the
  workspace's `.opencode/` directory on every regeneration.
- **7.5 - Place `scripts/{generated,user}`, `data/`, `scratch/` skeletons.**
  Create the runnable directories from the profile source. The
  `vibetrading/py/` Python client mirror was removed by the MCP runtime
  plan. The `scripts/user/` directory is created but never deleted.
- **7.6 - Add path-preservation rules.** Document and enforce that the
  generator must not delete `scripts/user/` or any other explicitly-writable
  path. Add a unit test that verifies a fake user-written script survives
  a regeneration.
- **7.7 - Wire workspace regeneration into agent create and update.** Call
  the generator when an OpenCode agent is created and when its
  `runtime_config` changes. Skip for Hermes agents.

### Phase 8: Backend Adapter Abstraction

- **8.1 - Define the `BackendKind` enum.** Add `BackendKind { Hermes,
  OpenCode }` in the agents module with `as_str`, `FromStr`, and sqlx
  `Type` derives.
- **8.2 - Define the `AgentBackend` trait.** Declare the methods needed to
  dispatch a run, record a backend run ref, and report runtime health.
  Keep it small.
- **8.3 - Define `DispatchRequest` and `DispatchResult` types.** Plain
  structs. The result should carry a `backend_run_ref` plus a status enum.
- **8.4 - Implement `HermesBackend`.** Wrap the existing `HermesClient` and
  `AgentOrchestrator` paths. Since Hermes is externally scheduled, the
  adapter is mostly read-only and reports health from the existing
  `*_context_last_used_at` timestamps.
- **8.5 - Implement `OpenCodeBackend` as a mock.** Stand-in adapter that
  records `agentic_runs` rows and logs the would-be command and agent. It
  must satisfy the trait but must not make any real HTTP calls yet.

### Phase 9: Agentic Scheduler

- **9.1 - Add `src/agentic_scheduler/mod.rs` and `scheduler.rs` skeleton.**
  New module with `AgenticScheduler::new` and a `run` loop. Not yet wired
  into `main.rs`.
- **9.2 - Implement the due-time loop.** Walk enabled schedules for enabled
  OpenCode agents and check `now >= next_run_at`.
- **9.3 - Implement concurrency enforcement.** Before dispatch, check for
  an active run on the same schedule; if present, insert a skipped run and
  advance `next_run_at`.
- **9.4 - Implement queued-run insertion and dispatch.** Insert a `queued`
  row, advance `next_run_at` to `now + interval_seconds`, mark the run
  `running`, then call the adapter. Move the actual `running` transition
  inside the dispatch path.
- **9.5 - Implement timeout and failure marking.** After dispatch, mark
  the run `succeeded`, `failed`, or `aborted` based on the adapter result.
  Record a short `error_summary` on failure.
- **9.6 - Add unit tests for scheduling semantics.** Cover: due run
  dispatches, active run is skipped, failed run is marked, disabled
  schedule is skipped, disabled parent agent is skipped.
- **9.7 - Wire `AgenticScheduler` into `main.rs`.** Start it next to the
  existing `AgentOrchestrator`. Both should share the same shutdown signal.

### Phase 10: Real OpenCode Backend

- **10.1 - Add an OpenCode HTTP client.** Use `reqwest` to call the OpenCode
  server. Place it in `src/agent_runtime/opencode/client.rs`. Support basic
  auth via the configured password.
- **10.2 - Implement real session creation.** Call `POST /session?directory={workspace_container_path}`
  with the per-agent workspace path as the `?directory=` query parameter. The
  server stores the directory on the session and returns a `Session` object
  whose `directory` and `path` fields reflect the binding. Capture and return
  the session id.
- **10.3 - Implement real command dispatch.** Send the resolved OpenCode
  command (analysis or trading) along with the operator prompt via
  `POST /session/{id}/command` or `POST /session/{id}/message`. **Do not
  repeat the `?directory=` query parameter** on these calls — the session is
  already permanently bound to the workspace, and passing a different
  directory is silently ignored. The directory binding cannot be changed for
  an existing session; to switch directories, create a new session.
- **10.4 - Implement session status polling.** Poll the session until it
  reaches a terminal state or the schedule's `timeout_seconds` elapses,
  then mark the run accordingly.
- **10.5 - Replace the mock `OpenCodeBackend` with the real one.** Remove
  the mock from production paths; keep it behind a test helper for now.
- **10.6 - Add health reporting.** Expose an `is_healthy` method that
  returns true when the OpenCode server responds to a basic health
  endpoint.

### Phase 11: Analysis Jobs

- **11.1 - Flesh out `commands/vibetrading-analysis.md`.** Describe the
  analysis loop: read latest account state, read relevant memories,
  optionally write a Python analysis script under `scripts/user/`, write a
  structured memory.
- **11.2 - Flesh out `agents/analysis.md`.** Describe the analysis agent's
  permissions: may read everything, may write memories, may write Python
  under `scripts/user/`, must not place or cancel orders.
- **11.3 - ~~Add the `vibetrading_client` Python wrapper.~~** Removed by
  the MCP runtime plan. Analysis agents now use the `vibetrading` MCP
  tools (`list_memories`, `get_latest_analysis`, `write_memory`, etc.)
  instead of a workspace-local Python client.
- **11.4 - Default `analysis-15m` schedule seed.** When an OpenCode agent
  is created, insert a default `analysis-15m` schedule with the recommended
  interval and timeout.
- **11.5 - End-to-end test: analysis job writes a memory.** Run the
  analysis command against a sandbox agent, verify a memory row appears in
  Postgres, and verify the run row is `succeeded` with a
  `backend_run_ref`.

### Phase 12: Trading Jobs (Dry-Run)

- **12.1 - Flesh out `commands/vibetrading-trading.md`.** Describe the
  trading loop: read latest account state, read active memories, propose
  orders through the Vibetrading tool, never sign directly.
- **12.2 - Flesh out `agents/trading.md`.** Describe the trading agent's
  permissions: may read account state and memories, may call the
  `propose_order` tool, must not write Python scripts in the initial
  design.
- **12.3 - Add the `propose_order` Vibetrading tool.** A new
  `POST /api/v1/order-proposals` endpoint that records the proposal in
  Postgres but never signs or submits. Returns the stored proposal id.
- **12.4 - Add a `propose_order` method to the Python client.** Wire the
  trading command to the new endpoint.
- **12.5 - Default `trading-1m` schedule seed.** Insert a default
  `trading-1m` schedule with the recommended interval and timeout when an
  OpenCode agent is created.
- **12.6 - End-to-end test: trading job proposes orders only.** Run the
  trading command in dry-run mode, verify a proposal row is created in
  Postgres, and verify that no order is submitted to Hyperliquid.

### Phase 13: Real Trading

- **13.1 - Wire real `submit_order` and `cancel_order` tools.** Reuse the
  existing `/api/v1/orders` and `/api/v1/orders/:id/cancel` endpoints;
  ensure the OpenCode path uses the same auth and instrument-restriction
  rules as the Hermes path.
- **13.2 - Add a per-agent kill switch.** A boolean on the registry row,
  default false, that, when set, blocks the `AgenticScheduler` from
  dispatching new runs. Add a UI toggle.
- **13.3 - Verify Hyperliquid private keys never reach OpenCode.** Add a
  test that scans the OpenCode workspace files for any hex string that
  parses as a private key. Add the same scan to CI for
  `agent-runtime/opencode/` and the workspace generator output.
- **13.4 - Document enabling real trading in production.** Update
  `OpenCode.md` and `Backends.md` with the explicit acknowledgement that
  real trading is now enabled, the kill switch procedure, and the rollback
  path.

### Phase 14: UI

- **14.1 - Backend badge on `/agents` list.** Show `Hermes` or `OpenCode`
  next to each agent as a small chip.
- **14.2 - Mandatory runtime picker on `/agents/new`.** The create-agent
  form must require selecting an existing `agent_runtime` row. Reject the
  submit when no runtime is selected.
- **14.3 - Per-backend setup instructions.** After a runtime is selected
  on `/agents/new`, show a small help block describing the runtime's setup
  expectations (Hermes profile install for Hermes, container env vars for
  OpenCode).
- **14.4 - Runtime registration page.** Add `/runtimes` and
  `/runtimes/new` for creating `agent_runtime` rows.
- **14.5 - OpenCode schedule management UI.** Add a "Schedules" tab to
  the OpenCode agent detail page. List, enable, disable, and edit
  `agentic_job_schedules` rows.
- **14.6 - OpenCode run history UI.** Add a "Runs" tab to the OpenCode
  agent detail page. Paginate `agentic_runs` rows with status, duration,
  and a link to the OpenCode plugin session.
- **14.7 - Backend filter on `/agents` list.** Allow operators to filter
  the list by `backend_kind` so they can isolate Hermes or OpenCode
  agents.

### Cross-Cutting

- **X.1 - Add a small "Rollout Stages" appendix to `OpenCode.md`.** Mirror
  the phase structure of this TODO list so operators can see at a glance
  where the implementation is. Keep it short.
- **X.2 - Remove resolved items from "Deferred Investigation Items".**
  Once 1.1 through 1.4 are answered, the deferred section should be empty
  or contain only genuinely open questions.

## OpenCode HTTP API Notes

Concrete facts verified by running requests against the local OpenCode
`1.17.11` server (`opencode-local` runtime, `http://localhost:14096`) against
the generated `/workspaces/agents/btc-2` workspace. These are the
contract that the `OpenCodeBackend` adapter in `src/agent_runtime/opencode/`
must implement; the deferred items below reference back to them.

### Directory is a query parameter, not a header

There is **no `x-opencode-directory` header** in the OpenCode `1.17.11` HTTP
API. The per-request project directory is a `?directory=<abs-path>` query
parameter. It appears as an optional parameter on essentially every
project-scoped route, including:

- `POST /session` — bind a new session to the directory
- `GET /session` — list sessions (filter by directory)
- `POST /session/{id}/message` — send a message
- `POST /session/{id}/message/{messageID}` — fetch a message
- `GET /session/{id}/message` — list messages
- `POST /session/{id}/command` — run a slash command
- `POST /session/{id}/prompt_async` — fire-and-forget message
- `GET /agent`, `GET /command`, `GET /config`, `GET /provider`,
  `GET /file`, `GET /find`, `GET /path`, `GET /project/current`, etc.

Most discovery and project introspection calls only need the `?directory=`
parameter when the project is not yet known. Once a session is created, the
directory is permanently bound and follow-up calls do not need to repeat it.

A sibling `?workspace=<name>` parameter scopes the call to a named workspace
inside a worktree, but Vibetrading uses one workspace per agent, so the
adapter only needs `?directory=`.

### Session directory binding is permanent

`POST /session?directory=/workspaces/agents/btc-2` returns:

```json
{
  "id": "ses_0fa0a4ad4ffeJ8MjoTfupnehRM",
  "directory": "/workspaces/agents/btc-2",
  "path": "workspaces/agents/btc-2",
  "projectID": "global",
  "title": "btc-2 directory test",
  ...
}
```

The returned session object carries the `directory` and `path` fields. All
subsequent `/message`, `/command`, and `/prompt_async` calls on that session
id are automatically scoped to that directory, **whether or not the caller
passes `?directory=`**. The adapter does not need to remember or re-send the
directory; it just stores the session id as `agentic_runs.backend_run_ref`.

The binding is one-way: there is no API to change the directory of an
existing session. Passing a different `?directory=` value on a follow-up
message call is silently ignored. To run against a different workspace, the
adapter must create a new session.

### Verified request/response shape

```text
# Create a session scoped to a workspace
POST /session?directory=/workspaces/agents/btc-2
Content-Type: application/json
{
  "title": "btc-2 directory test"
}

→ 200 OK
{
  "id": "ses_…",
  "directory": "/workspaces/agents/btc-2",
  "path": "workspaces/agents/btc-2",
  "projectID": "global",
  ...
}

# Send a message (no ?directory= required; cwd stays bound)
POST /session/ses_…/message
Content-Type: application/json
{
  "model": { "providerID": "openrouter", "modelID": "google/gemini-2.5-flash" },
  "parts": [{ "type": "text", "text": "What is your cwd?" }]
}

→ 200 OK
{
  "info": {
    "role": "assistant",
    "agent": "build",
    "path": { "cwd": "/workspaces/agents/btc-2", "root": "/" },
    "finish": "stop",
    ...
  },
  "parts": [{ "type": "text", "text": "/workspaces/agents/btc-2" }, ...]
}

# Run a slash command (vibetrading-analysis, vibetrading-trading, etc.)
POST /session/ses_…/command
Content-Type: application/json
{ "command": "vibetrading-analysis", "arguments": "…" }

→ 200 OK
{ "info": { "agent": "build", "path": { "cwd": "/workspaces/agents/btc-2", "root": "/" }, "finish": "stop" }, "parts": [...] }
```

`/session/{id}/prompt_async` takes the same body as `/message` and returns
`204 No Content` immediately, which is the right shape for the
`AgenticScheduler` dispatch path when the adapter is not the one polling for
completion. If the adapter wants the synchronous response, use `/message`.

### Discovery endpoints the adapter can use

- `GET /global/health` → `{ "healthy": true, "version": "1.17.11" }` for
  the `is_healthy` check in 10.6. Does not need auth on most builds but
  passes Basic auth when set.
- `GET /provider?directory={path}` → list connected providers, their
  models, and the `default` model map. The adapter should pick the
  per-schedule `model_provider_id` / `model_id` from
  `agentic_job_schedules`, not from these defaults.
- `GET /agent?directory={path}` → list available OpenCode agents
  (`build`, `analysis`, `trading`, `plan`, ...). The adapter picks
  `analysis` or `trading` based on the resolved `job_kind`.
- `GET /command?directory={path}` → list available slash commands
  (`/init`, `/review`, `/vibetrading-analysis`, `/vibetrading-trading`,
  ...). The adapter picks `vibetrading-analysis` or
  `vibetrading-trading` based on `job_kind`.
- `GET /session/{id}/message` → full transcript with parts; useful for the
  post-run database plugin mirror and for surfacing run output to the UI.

### Auth and CORS

The server uses HTTP Basic auth with `OPENCODE_SERVER_USERNAME` (default
`opencode`) and `OPENCODE_SERVER_PASSWORD`. The adapter must send an
`Authorization: Basic …` header on every request. The `opencode` CLI itself
is a valid Basic-auth client and can be used for manual testing:

```sh
AUTH=$(printf 'opencode:%s' "$OPENCODE_SERVER_PASSWORD" | base64 -w0)
curl -H "Authorization: Basic $AUTH" \
     "http://localhost:14096/global/health"
```

CORS is opt-in via `--cors` on the opencode server; the Vibetrading backend
talks server-to-server so CORS is not needed for the adapter.

## Deferred Investigation Items

These should be answered before implementation:

- ~~whether `opencode-sdk-rs` supports setting `x-opencode-directory` per request~~
  Resolved by spike 1.1: there is no such header in OpenCode `1.17.11`; the
  directory is a `?directory=` query parameter, and the `opencode-sdk-rs`
  client does not currently expose it. The adapter uses raw `reqwest`
  requests. See "OpenCode HTTP API Notes" above.
- ~~whether raw HTTP is needed for directory-scoped session creation or command
  execution~~
  Resolved by spike 1.1: yes, raw HTTP is the right path for the adapter in
  the initial design. See "OpenCode HTTP API Notes" above.
- ~~whether `POST /session` binds the session permanently to the requested
  project directory~~
  Resolved by spike 1.1: yes, the binding is permanent. See "OpenCode HTTP
  API Notes" above.
- ~~whether follow-up session message and command calls also require the directory
  header~~
  Resolved by spike 1.1: no `?directory=` (and no header) is needed on
  follow-up calls, and the value would be ignored anyway. See "OpenCode HTTP
  API Notes" above.
- whether project-level OpenCode config reloads after generated files change
- how headless OpenCode behaves when a permission resolves to `ask`
- how to configure production permissions so runs never require interactive
  approval
- ~~whether OpenCode's database plugin can operate cleanly in the `opencode`
  Postgres schema~~
  Resolved by the OpenCode Database Plugin Foundation plan: the plugin's
  SQL is unqualified, and `OPENCODE_DATABASE_URL` is configured with
  `search_path=opencode,public` so the plugin's tables, indexes,
  triggers, function, and view are created and queried under the
  `opencode` schema without modifying the upstream DDL. See "OpenCode
  Database Plugin" above.
- where OpenCode stores its internal data in the container and how to force that
  path cleanly
- what cleanup or retention strategy is needed for OpenCode internal data and
  plugin tables
- whether OpenCode can safely receive per-agent Vibetrading API credentials at
  run time without exposing all agents' credentials globally

## OpenCode MCP Investigation (Vibetrading MCP Runtime)

The Vibetrading backend no longer ships a workspace-local Python API client.
OpenCode agents interact with the Vibetrading backend exclusively through the
`vibetrading` MCP server.

The following questions were investigated against the upstream OpenCode MCP
docs (`opencode.ai/docs/mcp-servers/`) and validated for the image version
`ghcr.io/anomalyco/opencode:1.17.11` used in `podman-compose.yaml`.

### Config file extension

OpenCode accepts both `opencode.json` and `opencode.jsonc` at the project
level. The MCP config syntax is identical between them. Generated Vibetrading
workspaces continue to use `opencode.json` (no comments needed) for simpler
machine generation.

### Local MCP server config shape

A project-local stdio MCP server is registered under `mcp` with a unique name:

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "vibetrading": {
      "type": "local",
      "command": ["python", "/opt/vibetrading/mcp/server.py"],
      "cwd": ".",
      "enabled": true
    }
  }
}
```

Supported `local` options:

| Field         | Type    | Required | Notes                                                       |
|---------------|---------|----------|-------------------------------------------------------------|
| `type`        | String  | yes      | Must be `"local"`.                                          |
| `command`     | Array   | yes      | Command and arguments; the first entry is the executable.   |
| `cwd`         | String  | no       | Working directory. Relative paths resolve from workspace.   |
| `environment` | Object  | no       | Env vars explicitly passed to the MCP server process.      |
| `enabled`     | Boolean | no       | Defaults to `true`; can disable without removing config.    |
| `timeout`     | Number  | no       | Tool-fetch timeout in ms. Default is 5000.                  |

### Per-workspace process lifecycle

Project-level MCP entries launch a separate stdio MCP process per
project/session, not one global server-wide daemon. Each generated Vibetrading
agent workspace therefore gets its own authenticated MCP process, isolated by
its workspace `.env`.

### Environment inheritance

The OpenCode server process is the parent of every MCP process. The MCP
process inherits the parent process's environment unless a config `environment`
override replaces specific values. The dotenv plugin
(`@aeondave/opencode-dotenv`) loads the workspace `.env` into the OpenCode
server's process environment; whether those variables are passed through to
the spawned MCP child process is not documented as a guarantee.

To remove that uncertainty, the Vibetrading MCP server loads the workspace
`.env` itself from its current working directory (the workspace), which is set
explicitly by the `cwd` config field. This keeps auth credentials scoped to the
workspace regardless of any parent-process inheritance quirks.

### Conclusion

The expected preferred result from the plan is satisfied:

- Project-local config launches a separate MCP process for each agent
  workspace.
- The MCP server loads `.env` from the workspace via its own `cwd`.

The custom OpenCode image is required only to ship the MCP server source at a
fixed path (`/opt/vibetrading/mcp/`) and to install the MCP Python
dependencies in the container.
