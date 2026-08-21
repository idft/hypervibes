---
slug: /concepts/jobs
---

# Jobs

Jobs are the durable schedules and follow-up tasks that run an agent's
OpenCode sessions. HyperVibes stores both the job configuration and each run,
so users can inspect work that is queued, running, completed, or failed.

## Job types

Each new OpenCode agent receives seven disabled jobs: three analysis schedules,
one trading schedule, one daily review schedule, and two follow-up hooks.

| Job | Default schedule or trigger | Purpose |
| --- | --- | --- |
| Analysis | Candle close at 15m, 1h, or 1d | Runs strategy analysis for selected markets and saves the results as memory. |
| Market analysis | `analysis_batch_completed` | Combines the latest analysis memories into a market-level execution handoff. |
| Trading | Candle close at 5m | Reviews the current market-analysis handoff, places or manages orders, and manages exits. |
| Daily review | Candle close at 1d | Reviews agent performance and records accumulated learnings. |
| Analysis coding | `daily_review_completed` | Generates or improves user analysis code after a qualifying review. This hook is disabled by default and request-gated. |

Candle-close jobs are driven by UTC candle boundaries after the configured
settling delay. The two event triggers are direct follow-ups from their
predecessor; they are not a separate durable event queue.

## Configure a job

The Jobs tab lets a user configure each job's:

- enabled state
- provider, model, and optional thinking mode
- timeout
- Additional Instructions

An enabled scheduled job must have an explicit model. Additional Instructions
are specific to that job and are appended to its selected strategy prompt; they
are not part of the saved strategy prompt and are not exposed through the agent
API or MCP.

Analysis, market analysis, trading, and daily review can use reusable analysis
code but cannot modify `scripts/user/`. Analysis coding is the only job allowed
to change that tree, and it requires an explicit strong provider and model.

## Dispatch and runs

The scheduler claims due jobs transactionally and dispatches them through the
OpenCode backend. Before a run starts, HyperVibes snapshots its provider,
model, optional thinking mode, selected instruments, job-specific prompt,
latest agent learnings, global prompt, and trading account snapshot when
applicable.

The run keeps its HyperVibes status independently from the OpenCode session.
On startup and during periodic recovery, stale active runs are reconciled so an
interrupted process does not permanently block later work for the same agent.
Run details include the OpenCode transcript, tool activity, errors, token and
context telemetry, and cost when those records are available.

Jobs are held while per-agent workspace maintenance is queued or running. A
maintenance task waits for active runs and busy OpenCode sessions to finish;
then scheduled work remains due and can run after maintenance completes.

Workspace maintenance includes regular regeneration and hard resets. Regular
regeneration refreshes generated files while preserving user-managed files in
`scripts/user/`, `data/`, and `scratch/`. A hard reset removes the complete
workspace before regenerating it. Only one maintenance task can be queued or
running for an agent, and the agent settings page reports drift in
template-managed files.

See [Prompts](Prompts.md) for the content passed to each job and
[Architecture](/docs/development/architecture) for the runtime lifecycle.
