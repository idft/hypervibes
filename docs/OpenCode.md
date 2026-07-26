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

## Provider Connections

Provider credentials are global to the one shared OpenCode service. Operators
manage API-key and OAuth connections at `/providers`; credentials are never
stored on an agent or in Vibetrading's database. OpenCode persists its auth
storage in the retained `opencode_data` volume.

The page uses OpenCode's advertised authentication methods rather than a fixed
provider list or hard-coded method indexes. For OpenAI, select
`ChatGPT Pro/Plus (headless)`, open the displayed device URL, follow its
instructions, and then submit the completion action in Vibetrading. The browser
loopback method is disabled because it requires a local OpenCode TUI. Other
providers may advertise API-key prompts, OAuth prompts, or both.

Provider API-key environment variables are no longer part of the Compose
runtime. During migration, connect and verify providers through `/providers`
before recreating OpenCode without those fallback variables. Do not delete the
`opencode_data` volume.

Providers that declare an API-key environment variable but do not advertise
auth methods through `/provider/auth` (such as `ollama-cloud`) are offered a
generic "Manually enter API key" connection method in the UI.

## Config Reload

OpenCode caches its provider list in memory. After a provider is connected or
disconnected through `/providers`, the cached `connected` list does not update
until OpenCode's instance cache is disposed. Vibetrading handles this
automatically by queuing a `provider_config_reload` maintenance task after
every successful connect, OAuth callback, or disconnect.

The reload task waits until no OpenCode sessions are active (`busy`/`retry`)
and then calls `POST /global/dispose`, which clears OpenCode's in-memory
instance cache. The next `/provider` request rebuilds the cache from the
updated `auth.json`. This does not delete sessions from the database, but it
does interrupt any in-flight inference, so the task only runs when all agents
are idle. The `/providers` page shows a pending/running reload banner with a
manual "Reload config now" button.

Removing an auth record removes OpenCode's stored credential only. The config
reload ensures the removal takes effect for new provider initialization.
Provider configuration editing, custom base URLs, model allowlists, and
default-model editing are intentionally out of scope.

The OpenCode server password is required, is used for Basic authentication, and
must be rotated if exposed. The web UI does not log or render Basic-auth values,
API keys, OAuth codes, prompt answers, or provider token data.

## Runtime Model

OpenCode is a single application-level dependency rather than a database-
managed runtime assignment. The endpoint is configured with
`OPENCODE_BASE_URL`, defaulting to `http://localhost:14096` for a host-running
Vibetrading process. An application container on the Compose network should
use `http://opencode:14096`.

Each agent has one generated workspace. Workspace paths and generation
metadata are stored in `agents.runtime_config`; no OpenCode endpoint or runtime
identity is stored on the agent row.

OpenCode's provider discovery request requires a workspace directory, but model
providers and models are configured by the shared OpenCode backend. Vibetrading
uses the configured container workspace root to warm one application-wide,
backend-URL-keyed provider cache at startup, including before any agents exist.
Model pickers wait for an initial discovery instead of rendering an empty
selector; later pickers reuse that cache for its five-minute lifetime.

## Agent Workspaces

Creating an agent generates a workspace under the configured host workspace root.

New agent creation refuses to reuse an existing stale workspace directory for the same derived agent key.

The generated workspace includes:

- `opencode.json`
- `AGENTS.md`
- `.opencode/`
- writable `scripts/user/`, `data/`, and `scratch/` paths

The generated workspace template now includes dedicated OpenCode agent/command files for:

- `analysis`
- `market-analysis`
- `trading`
- `daily-review`

The workspace `.env` is backend-owned generated state and should not be read or modified by agents.

## Agent Conversations

The Chat tab creates persistent `agent_conversations` mappings to dedicated
OpenCode sessions. They are independent of scheduled jobs and use the selected
provider/model only for later turns; OpenCode keeps per-message model
attribution. The UI mirrors transcript, tool, error, token/context, cost, and
compaction telemetry through server-sent events. Compact and deletion are
OpenCode-owned session operations and are available only when the session is
idle.

The `agent-conversations` profile is defined in the container-global
`agent-runtime/container/opencode.jsonc` and duplicated in the workspace
template. It uses Vibetrading MCP tools for data, denies native
shell/filesystem access, and never reads `.env`. Once the OpenCode image is
rolled out, existing workspaces can use the profile without regeneration.

Order and memory-write permissions are saved per conversation as Deny, Confirm,
or Allow. Confirm maps to an OpenCode permission request, so the MCP call waits
for the operator's one-time approval rather than relying on an app-only toggle.

Re-generating a workspace is now queued as per-agent maintenance work instead of running inline in the settings POST handler.

Regular re-generation still refreshes generated files while preserving user-managed files under paths like `scripts/user/`, `data/`, and `scratch/`.

Operators may also queue a hard reset, which deletes the full workspace directory first and then re-generates it from the template.

Only one queued/running workspace maintenance task is allowed per agent. Duplicate submissions are rejected.

The agent settings page shows whether the on-disk workspace has drifted from `agent-runtime/workspace-template/` and lists changed template-managed files with per-file line counts.

That drift check only compares template-derived files such as `AGENTS.md`, `opencode.json`, and `.opencode/...`. It does not inspect agent-created files under `scripts/user/`, `data/`, or `scratch/`.

Deleting an OpenCode agent deletes its generated workspace directory after the database delete succeeds.

## Scheduling

OpenCode jobs are scheduled by Vibetrading.

Current built-in job kinds are:

- `analysis`
- `market_analysis` hook
- `trading`
- `daily_review`
- `analysis_coding` hook

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

The OpenCode database plugin's `tool_executions` completion fields can be
incomplete because plugin writes are asynchronous. Run-detail transcripts use
completed tool state from `opencode.message_parts` when available and retain
`tool_executions` as a fallback for older sessions.

Run-detail pages display `opencode.sessions`, `opencode.messages`,
`opencode.message_parts`, `opencode.tool_executions`, and
`opencode.session_errors`. The displayed session is associated with a
Vibetrading run through `agentic_runs.backend_run_ref`. Plugin writes and
Vibetrading run updates issue Postgres notifications after their transactions
commit; the web process fans those notifications out to the run-detail SSE
stream and the Jobs tab's Recent Runs SSE section. Each Recent Runs update
re-queries and re-renders the complete section so inserts, status transitions,
backend references, errors, and pagination changes are reflected. Run-detail
updates re-render the complete summary and transcript partial, because plugin
records are mutable and may arrive asynchronously rather than as append-only
rows. OpenCode session notifications remain relevant only to the run-detail
transcript and summary.

Scheduled jobs and hooks must have an explicit model selected before they can be enabled from the operator UI.

## Analysis Coding

Analysis coding is a strong-model, request-gated maintenance job. It is
disabled by default and must have an explicit provider/model selection. Manual
runs are permitted even while the hook is disabled.

The worker copies `scripts/user/` into an isolated candidate workspace and
never lets the model edit the live workspace. Candidate changes are validated
by a fixed local MCP tool in the OpenCode analysis runtime. The validation
result is bound to the candidate tree hash, and the worker promotes only when
the final candidate still matches it. Successful promotion uses an exclusive
per-agent workspace write lease and an atomic JSON promotion journal; an
incomplete journal is reconciled at startup by restoring the last known-good
backup. Successful backups are retained as rollback versions, with the five
most recent versions kept per agent.

Analysis and trading sessions hold shared read leases for their full lane.
Coding generation does not hold the live write lease; only the final
promotion and promoted-tree hash verification do. Native OpenCode read, edit,
and glob permissions include workspace-relative rules plus an exact generated
candidate scope because non-Git OpenCode projects authorize file tools relative
to `/`. Both forms are limited to the current candidate's `scripts/user/` tree.
Pyright is installed with its bundled Node.js runtime so Python LSP diagnostics
do not depend on a writable runtime cache. The coding profile cannot place
orders or write ordinary memories.

Native focused edits avoid resending complete large files. The only dedicated
coding MCP tools are the privileged fixed validator and structured report
submission. Validation failures return bounded subprocess diagnostics.
Validation checks deterministic output,
non-empty finite measurements, strict open/future-candle rejection, sensitivity
to eligible candles, CLI/input context agreement, and every canonical
Hyperliquid interval. Python bytecode is redirected outside the candidate and
ignored artifacts are removed before promotion.

The canonical output requires `source_range.count` to equal the number of
eligible candles used. Bootstrap jobs are instructed to establish a minimal
one-candle-safe baseline before adding broader strategy indicators. Fixed
validation is local and fixture-based. It also verifies missing output-parent
creation, candle-order invariance, and known signal semantics such as deriving
last-candle body direction from close versus open. Invariance failures identify
the first changed output path. The agent must resolve every failed check and
receive `ok: true` before submitting its report.

The `analysis-coding` OpenCode agent explicitly invokes its dedicated
`analysis-coding` skill. The skill defines the canonical analysis CLI,
quantitative output envelope, optional indicator signals, closed-candle rule,
and focused optional-test policy. The fixed validator and its deterministic fixture are container-global assets at
`/opt/vibetrading/coding/`; they are not copied into agent workspaces.
# Workspace Lifecycle

Agent workspaces live in the OpenCode container's named volume, not on a host bind mount.
Vibetrading uses the authenticated workspace controller for generation, drift inspection,
coding candidates, report storage, promotion, and recovery. `opencode_data` remains a separate
named volume for global provider credentials and OAuth state.
