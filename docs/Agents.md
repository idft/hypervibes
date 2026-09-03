---
slug: /concepts/agents
---

# Agents

## Configure and operate an agent

An agent is an isolated HyperVibes configuration with one assigned trading
account, selected perpetual instruments, strategy prompts, a durable Coding
package, and scheduled sub-agents.

Before an agent can trade on a schedule, it needs:

- an active, enabled agent with an assigned main account or sub-account
- at least one selected perpetual instrument
- at least one enabled analysis schedule
- an enabled market-analysis follow-up sub-agent
- an enabled trading schedule
- an explicit model selection for each enabled scheduled sub-agent

HyperVibes reports incomplete setups as `Paused` or `Setup required`. It reports
`Enabled` only after the required configuration is in place.

The web interface separates analysis, market analysis, trading, and daily
review into durable sub-agents. Sub-agent runs record whether work is scheduled, running,
completed, or failed. Market analysis creates the execution handoff for
trading; trading ordinarily uses that handoff rather than independently
developing a new market thesis. The memory system preserves analyses,
handoffs, reviews, and learnings across runs.

The web interface shows positions, orders, transactions, memory, sub-agents, and
conversations. Missing or stale live account data is marked unavailable, and
new agent-originated exposure fails closed until the required live streams are
current. Before enabling live trading, verify the account, trading signer,
instruments, prompts, models, schedules, and current account state. Review runs
and positions continuously, especially after provider changes or configuration
edits.

## Coding Package

An agent's only durable filesystem state is its Coding package at
`packages/<agent-key>/` under the workspace root. It contains `manifest.json`
plus any coding-agent-defined files and is created by the first successful
coding promotion. There is no permanent per-agent workspace directory, no
permanent OpenCode project, and no per-agent `.env`.

Runs, coding candidates, and conversations are isolated workspaces with their
own lifetimes. A run copies the package root into its run-local
`scripts/user/` directory. The analysis agent may inspect and directly execute
package Python from that read-only copy; only the analysis-coding sub-agent can
change the durable package.

### Coding Page

The agent Coding page is a read-only inspection view of the durable Coding
package. It shows the package status (no package yet, a valid package with its
version and manifest hash, or an invalid package with a safe error), a file
browser with paths relative to the package root, and text file previews. It
never lists or reads `.env` files; binary and oversized files remain visible in
the tree with an unavailable preview state. The page also shows the latest
analysis-coding task status, including its phase, failure summary, and coding
report summary with changed paths. Coding tasks are queued through the
analysis-coding sub-agent's Run now action.

### Reset Memories

The agent settings page provides a synchronous **Reset memories** action that
deletes all memories stored for the agent after confirmation. It does not
change package files and does not queue a maintenance task.

## Instrument Selection

Instrument selection limits the Hyperliquid perpetual markets an agent can
analyze and trade. At least one instrument must be selected before the agent can
trade. With no instruments selected, the agent does not analyze markets or place
new trades.
