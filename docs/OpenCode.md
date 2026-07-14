# OpenCode Backend

Related docs:

- `README.md`
- `Agents.md`
- `Memory.md`
- `Hyperliquid.md`

## Goal

OpenCode is the only supported agent execution backend today.

Each generated workspace starts a local stdio MCP server. It reads the
workspace-scoped Vibetrading API credentials and adapts OpenCode tool calls to
the authenticated agent API; it never receives a Hyperliquid private key.

Vibetrading owns:

- agent identity
- job scheduling and dispatch
- run tracking
- memory, account state, and order execution

OpenCode owns:

- LLM execution
- tool execution
- session lifecycle inside the runtime server

## Runtime Model

The runtime model is:

- one `agent_runtimes` row per OpenCode server instance
- one `agents.runtime_id` link from each agent to its runtime
- one generated workspace per agent

The seeded runtime is:

- `opencode-local` -> `OpenCode local` -> `http://localhost:14096`

`backend_kind` remains part of the schema even though only `opencode` is currently valid.

## Agent Workspaces

Creating an agent generates a workspace under the configured host workspace root.

New agent creation refuses to reuse an existing stale workspace directory for the same derived agent key.

The generated workspace includes:

- `opencode.json`
- `AGENTS.md`
- `.opencode/`
- `scripts/generated/`
- writable `scripts/user/`, `data/`, and `scratch/` paths

The generated workspace template now includes dedicated OpenCode agent/command files for:

- `analysis`
- `market-analysis`
- `trading`
- `daily-review`

The workspace `.env` is backend-owned generated state and should not be read or modified by agents.

Re-generating a workspace is now queued as per-agent maintenance work instead of running inline in the settings POST handler.

Regular re-generation still refreshes generated files while preserving user-managed files under paths like `scripts/user/`, `data/`, and `scratch/`.

Operators may also queue a hard reset, which deletes the full workspace directory first and then re-generates it from the template.

Only one queued/running workspace maintenance task is allowed per agent. Duplicate submissions are rejected.

The agent settings page shows whether the on-disk workspace has drifted from `agent-runtime/workspace-template/` and lists changed template-managed files with per-file line counts.

That drift check only compares template-derived files such as `AGENTS.md`, `opencode.json`, `.opencode/...`, and `scripts/generated/...`. It does not inspect agent-created files under `scripts/user/`, `data/`, or `scratch/`.

Deleting an OpenCode agent deletes its generated workspace directory after the database delete succeeds.

## Scheduling

OpenCode jobs are scheduled by Vibetrading.

Current built-in job kinds are:

- `analysis`
- `market_analysis` hook
- `trading`
- `daily_review`

The `AgenticScheduler` claims due work, dispatches runs through the OpenCode backend adapter, and stores run state in Postgres.

Before claiming new work for an agent lane, Vibetrading reconciles stale active runs left behind by app restarts. A `running` run whose OpenCode session is recorded as `idle` after a command was created is marked `succeeded`; queued or running orphan rows that have exceeded their configured timeout are marked `failed`, regardless of whether they ever reached an OpenCode session. The `AgenticScheduler` also runs a periodic global recovery sweep (throttled to once a minute) that applies the same reconciliation across every agent, so a `running` run whose dispatch worker has died does not block its lane until the next claim attempt. This prevents one interrupted process from causing all later runs in the same lane to be skipped forever.

## Shutdown And Maintenance

A single signal handler in `main` watches for `SIGINT` (Ctrl-C) and `SIGTERM` and flips one shared `watch<bool>`. The web server, the `AgenticScheduler`, and the `HyperliquidAgentMonitor` all observe that flag and stop claiming new work. The web server's `axum::serve` `with_graceful_shutdown` future is driven by the same flag, so in-flight HTTP requests still finish.

The web server additionally waits for the shared `InFlightTracker` to drain (or hit the 30-minute grace) once the shutdown flag flips, because the agent's MCP server makes HTTP calls back into this API during a dispatch. Without that hold, the web server can return between agent tool calls and starve the in-flight dispatch's API calls (each MCP call from the agent would fail with connection refused).

Every dispatch is wrapped in an `InFlightTracker` guard that increments when the task starts and decrements on `Drop`. The scheduler and `main` both call `wait_idle_with_timeout(30m)` after the shutdown signal so any `run_command` HTTP call already in flight is given a chance to return naturally (or hit the schedule's own `timeout_seconds`). The 30-minute ceiling is `max schedule timeout (15m) + 15m buffer`; if it is hit, a warning is logged and the next start's recovery sweep will mark the affected runs as failed orphans.

A second `SIGINT` or `SIGTERM` flips a separate `force` watch (`force_shutdown_rx`). Both the web server's in-flight hold and the scheduler's drain wait observe it and return immediately, so the API is cut and the in-flight dispatches fail fast on their next MCP call (which then surfaces the timeout/error back through `run_command` and lets the process exit). After the second signal, further signals are ignored; use `kill -9` if a hard exit is required.

The new policy on what may and may not start after the shutdown signal:

- **Automatic scheduled dispatches** are not started. The scheduler's `tick` short-circuits when `shutdown_rx` is set, and the `for (agent_key, schedules) in by_agent` loop checks the flag between agents.
- **Manual "Run now" on a schedule** is rejected by the jobs route with a redirect that flashes a "server is shutting down" warning. The run row is inserted and immediately marked `failed` so the operator's intent is recorded.
- **Hook jobs (manual and automatic)** are still allowed to start after shutdown. Manual hook "Run now" runs even after the signal; the automatic `analysis_batch_completed` hook runs as part of its already-in-flight analysis lane. Both are tracked by the `InFlightTracker` so the drain logic awaits them.

Queued workspace maintenance is processed before normal schedule dispatch, but it only starts once the agent is fully idle:

- no queued/running `agentic_runs` remain for that agent
- no recorded OpenCode sessions for that workspace are marked `busy` or `retry`

While workspace maintenance is queued or running:

- scheduled jobs stay due and are held for later instead of being skipped
- manual job `Run now` and manual hook `Run now` are blocked with a warning

Automatic follow-up hooks for an analysis job that was already allowed to run are still queued and completed before maintenance starts.

The agent detail page exposes `Jobs` and `Prompts` tabs for OpenCode agents.

When a job is dispatched, Vibetrading builds the initial OpenCode command prompt with the agent metadata, selected instruments, the job-specific strategy prompt, the latest `agent_learnings` memory, operator prompt, and trading account snapshot when applicable.

Scheduled jobs and hooks must have an explicit model selected before they can be enabled from the operator UI.
