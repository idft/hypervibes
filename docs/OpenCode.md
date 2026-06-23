# OpenCode Agentic Runtime Plan

Related docs:

- `Agents.md` for the agent registry and account ownership model
- `Hermes.md` for the current Hermes profile distribution model being replaced
- `Memory.md` for analysis memory ownership and handoff semantics
- `Hyperliquid.md` for execution gateway and account sync ownership

## Goal

Replace Hermes as the agentic backend with an OpenCode server controlled by the
Vibetrading backend.

The intended ownership model is:

- Vibetrading owns agent identity, scheduling, run state, memory, account state,
  and order execution.
- OpenCode owns LLM/tool execution inside a containerized runtime.
- OpenCode sessions are invoked programmatically by the Vibetrading backend.
- Hyperliquid private keys remain backend-side and must not be exposed to
  OpenCode.

This is a major architecture change and should be implemented in phases. This
document captures the current plan, not a completed implementation.

## High-Level Architecture

The target shape is:

```text
Vibetrading backend
  -> app-owned agentic scheduler
  -> OpenCode HTTP API / Rust SDK
  -> OpenCode server container
  -> per-agent OpenCode project workspace
  -> Vibetrading APIs/tools for memory, account state, and execution
```

Runtime decisions so far:

- run one OpenCode server container for all agents
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

## Runtime Profile Source

Do not put trading-agent OpenCode commands, agents, or skills in the repository
root `.opencode/` directory. That directory is for local developer OpenCode
configuration.

The OpenCode runtime assets should live in a separate distribution-like source
directory, similar in purpose to the current Hermes profile distribution.

Proposed source layout:

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
├── vibetrading/
│   └── py/
└── requirements.txt
```

Generated agent workspaces may contain project-local `.opencode/` directories if
OpenCode requires that layout for command, agent, and skill discovery. The
source of truth should still be `agent-runtime/opencode/`, not the repository
root `.opencode/`.

## Agent Workspaces

Each Vibetrading agent should get its own OpenCode project directory under a
shared workspace volume.

Proposed generated layout:

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
├── vibetrading/
│   └── py/
├── data/
└── scratch/
```

Ownership rules:

- Vibetrading may create and update generated files.
- OpenCode agents may write only to explicitly writable paths.
- The analysis agent may write Python scripts under `scripts/user/`.
- The trading agent should not write Python scripts in the initial design.
- The backend must preserve agent-authored paths when regenerating runtime
  files.

The backend and OpenCode container should mount the workspace at the same path,
preferably `/workspaces`, to avoid path translation bugs.

Agent workspaces should be backed up. They are not intended to be disposable
because analysis agents may write durable Python analysis scripts.

Git should not be initialized in agent workspaces as part of the initial plan.

## Scheduler Model

The existing `AgentOrchestrator` is better understood as a Hyperliquid account
sync and monitoring task. The OpenCode migration should introduce a separate
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
  per enabled agent schedule:
    due-time calculation
    OpenCode session creation
    run status tracking
    timeout/failure handling
```

The current orchestrator may be renamed or moved later to better reflect its
Hyperliquid-specific responsibilities.

## DB-Backed Job Schedules

Agentic jobs should be DB-backed and configurable per agent. Do not hardcode only
one analysis loop and one trading loop.

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

Initial mapping:

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

Vibetrading should store canonical run lifecycle state in its own tables. Full
transcripts, tool calls, token usage, and cost data should come from the
OpenCode database plugin rather than being duplicated in Vibetrading run rows.

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
  opencode_session_id
  opencode_project_path
  opencode_agent
  opencode_command
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

Even though `opencode_agent` and `opencode_command` are derived from `job_kind`,
the resolved values should be copied into run rows for historical debugging.

Do not copy large run context blobs into `agentic_runs` by default. Link runs to
OpenCode plugin data with `opencode_session_id`.

Skipped runs should be inserted, not merely logged. A skipped run represents a
due schedule that did not start because another run for the same schedule was
already active.

## Scheduling Semantics

For each enabled schedule whose parent agent is enabled:

```text
if now >= next_run_at:
  if active run exists for the same schedule:
    insert skipped run
    advance next_run_at
  else:
    insert queued run
    advance next_run_at
    dispatch OpenCode run
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

The plugin schema should live under an `opencode` Postgres schema.

The plugin SQL should be vendored into this repository's migrations or migration
support files rather than requiring operators to fetch schema files manually at
runtime.

If the plugin SQL uses unqualified table names, test whether setting the
Postgres `search_path` to `opencode,public` is sufficient. If not, patch or fork
the schema/plugin as needed.

Plugin logging failure should not stop a trading or analysis run, but the system
should surface degraded observability.

## Configuration And Deployment

OpenCode should run in a custom container image based on the upstream OpenCode
image.

The image should include:

- OpenCode
- Python
- `uv`
- common Python analysis dependencies as needed
- Vibetrading Python client/runtime scripts
- OpenCode database plugin

Runtime volumes:

```text
/opencode-data
/workspaces
```

Provider credentials should be passed to the OpenCode container with environment
variables when possible.

OpenCode server auth should be enabled, even on the internal container network.

Agent-scoped Vibetrading API credentials should be treated differently from
provider credentials. Avoid making every agent's Vibetrading API key globally
available to all OpenCode sessions if possible. Prefer per-agent project config,
run context, or scoped tool configuration.

## Deprecated Hermes Concepts

The Hermes `job-context` check-in timestamps become unnecessary once
Vibetrading owns scheduling.

The following agent columns should eventually be deprecated or removed from the
OpenCode path:

```text
analysis_context_last_used_at
trading_context_last_used_at
analysis_prompt
trading_prompt
```

The prompt columns are replaced by `agentic_job_schedules.operator_prompt`, which
allows separate instructions for multiple analysis or trading schedules.

The check-in timestamp columns are replaced by `agentic_runs` and schedule status
derived from run history.

## Migration Direction

Existing Hermes agent state does not need to be migrated into the OpenCode
system. The OpenCode runtime can start fresh.

Suggested implementation phases:

1. Spike OpenCode server directory-scoped project behavior.
2. Add OpenCode container configuration for local development.
3. Add `agentic_job_schedules` and `agentic_runs` tables.
4. Vendor the OpenCode database plugin schema under the `opencode` schema.
5. Add the OpenCode runtime profile source directory.
6. Add per-agent workspace generation.
7. Add a scheduler with a mock runtime.
8. Wire the scheduler to OpenCode session and command execution.
9. Enable analysis jobs first.
10. Enable trading jobs in dry-run/proposal mode.
11. Enable real trading tools last.

## Deferred Investigation Items

These should be answered before implementation:

- whether `opencode-sdk-rs` supports setting `x-opencode-directory` per request
- whether raw HTTP is needed for directory-scoped session creation or command
  execution
- whether `POST /session` binds the session permanently to the requested
  project directory
- whether follow-up session message and command calls also require the directory
  header
- whether project-level OpenCode config reloads after generated files change
- how headless OpenCode behaves when a permission resolves to `ask`
- how to configure production permissions so runs never require interactive
  approval
- whether OpenCode's database plugin can operate cleanly in the `opencode`
  Postgres schema
- where OpenCode stores its internal data in the container and how to force that
  path cleanly
- what cleanup or retention strategy is needed for OpenCode internal data and
  plugin tables
