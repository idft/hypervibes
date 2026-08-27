---
slug: /concepts/sub-agents
---

# Sub-agents

Sub-agents are the durable schedules and follow-up tasks an agent dispatches to
run OpenCode sessions. HyperVibes stores both the sub-agent configuration and each run,
so users can inspect work that is queued, running, completed, or failed.

## Sub-agent types

Each new OpenCode agent receives seven disabled sub-agents: three analysis schedules,
one trading schedule, one daily review schedule, and two follow-up hooks.

| Sub-agent | Default schedule or trigger | Purpose |
| --- | --- | --- |
| Analysis | Candle close at 15m, 1h, or 1d | Runs strategy analysis for selected markets and saves the results as memory. |
| Market analysis | `analysis_batch_completed` | Combines the latest analysis memories into a market-level execution handoff. |
| Trading | Candle close at 5m | Reviews the current market-analysis handoff, places or manages orders, and manages exits. |
| Daily review | Candle close at 1d | Reviews agent performance and records accumulated learnings. |
| Analysis coding | On demand | Generates or improves user analysis code after a daily-review request, an approved Chat request, or a manual run. It is disabled by default. |

Candle-close sub-agents are driven by UTC candle boundaries after the configured
settling delay. Market analysis is a direct analysis follow-up. Analysis coding
is queued on demand and never waits for the requesting session.

## Configure a sub-agent

The Sub-agents tab lets a user configure each sub-agent's:

- enabled state
- provider, model, and optional thinking mode
- timeout
- Additional Instructions

An enabled scheduled sub-agent must have an explicit model. Additional Instructions
are specific to that sub-agent and are appended to its selected strategy prompt; they
are not part of the saved strategy prompt and are not exposed through the agent
API or MCP.

Analysis, market analysis, trading, and daily review can use reusable analysis
code but cannot modify `scripts/user/`. Analysis coding is the only sub-agent allowed
to change that tree, and it requires an explicit strong provider and model.

The initial notification capability default permits `hypervibes_send_notification`
only for trading. Analysis, market analysis, daily review, and analysis coding are
denied until their per-sub-agent capability controls are introduced.

## Dispatch and runs

The scheduler claims due sub-agents transactionally and dispatches them through the
OpenCode backend. Before a run starts, HyperVibes snapshots its provider,
model, optional thinking mode, selected instruments, sub-agent-specific prompt,
latest agent learnings, global prompt, and trading account snapshot when
applicable.

The run keeps its HyperVibes status independently from the OpenCode session.
On startup and during periodic recovery, stale active runs are reconciled so an
interrupted process does not permanently block later work for the same agent.
Run details include the OpenCode transcript, tool activity, errors, token and
context telemetry, and cost when those records are available.

Sub-agents are held while per-agent workspace maintenance is queued or running. A
maintenance task waits for active runs and busy OpenCode sessions to finish;
then scheduled work remains due and can run after maintenance completes.

Workspace maintenance includes regular regeneration and hard resets. Regular
regeneration refreshes generated files while preserving user-managed files in
`scripts/user/`, `data/`, and `scratch/`. A hard reset removes the complete
workspace before regenerating it. Only one maintenance task can be queued or
running for an agent, and the agent settings page reports drift in
template-managed files.

See [Prompts](Prompts.md) for the content passed to each sub-agent and
[Architecture](/docs/development/architecture) for the runtime lifecycle.
