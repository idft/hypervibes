---
slug: /concepts/sub-agents
---

# Sub-agents

Sub-agents are durable OpenCode jobs. HyperVibes persists each job's
configuration and runs so queued, running, completed, and failed work can be
inspected.

## Roles

Each new agent receives five disabled sub-agents: three independent Analysis
jobs and singleton Trading and Review jobs.

| Role | Default schedule | Purpose |
| --- | --- | --- |
| Analysis | Candle close at 15m, 1h, or 1d | Uses approved data tools for research and publishes discoverable memories. |
| Trading | Candle close at 5m | Synthesizes Analysis context, records decisions, and manages orders. |
| Review | Candle close at 1d | Reviews outcomes, records learnings, and can revise Trading and Analysis prompts when granted the capability. |

Candle-close jobs run at UTC candle boundaries after the configured settling
delay. Analysis and Trading retain the selected-instrument gate; Review does not
use that gate.

Analysis has many independently keyed jobs. Defaults use `technical-15m`,
`technical-1h`, and `technical-1d`; user-created jobs provide a unique bounded
ASCII-slug key and may share a schedule or timeframe with another job. Each
Analysis sub-agents own independent full prompt revisions but never modify
prompts themselves. Review is the sole scheduled role that can submit prompt
revisions.

## Configuration

The Analysis, Trading, and Review pages configure each role's enabled state,
provider, model, optional thinking mode, timeout, prompt, and capabilities.
Enabled scheduled jobs require an explicit model. The prompt is the complete
strategy input saved for that sub-agent.

Analysis uses its approved data tools and publishes research memories. It does
not author reusable code.

Capabilities are named assignments and are snapshotted with a run. Notifications
are available to every role. Analysis may optionally manage the Trading
instrument allowlist, while Review may optionally submit prompt revisions and
create or update indicators. Review can revise Trading and Analysis prompts but
never its own prompt. Capability changes apply only to future runs.

## Dispatch and runs

Before dispatch, HyperVibes snapshots the provider, model, thinking mode,
selected instruments, sub-agent prompt revision, latest learnings, and global
prompt. Trading also receives a live account snapshot. Run status remains
independent of the OpenCode session. Recovery reconciles stale active runs by
probing OpenCode: a confirmed missing session fails its run promptly, while an
unavailable OpenCode service leaves the run retryable until a later probe or
the normal timeout.

Scheduled work remains queued until its trigger and is independent of other
agents' runs. Provider configuration reloads are handled separately from
sub-agent runs.

See [Prompts](Prompts.md) and [Architecture](/docs/development/architecture).
