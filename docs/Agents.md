---
slug: /concepts/agents
---

# Agents

## Configure and operate an agent

An agent is an isolated HyperVibes configuration with one assigned trading
account, selected perpetual instruments, strategy prompts, an OpenCode
workspace, and scheduled jobs.

Before an agent can trade on a schedule, it needs:

- an active, enabled agent with an assigned main account or sub-account
- at least one selected perpetual instrument
- at least one enabled analysis schedule
- an enabled market-analysis follow-up job
- an enabled trading schedule
- an explicit model selection for each enabled scheduled job

HyperVibes reports incomplete setups as `Paused` or `Setup required`. It reports
`Enabled` only after the required configuration is in place.

The web interface separates analysis, market analysis, trading, and daily
review into durable jobs. Job runs record whether work is scheduled, running,
completed, or failed. Market analysis creates the execution handoff for
trading; trading ordinarily uses that handoff rather than independently
developing a new market thesis. The memory system preserves analyses,
handoffs, reviews, and learnings across runs.

The web interface shows positions, orders, transactions, memory, jobs, and
conversations. Missing or stale live account data is marked unavailable, and
new agent-originated exposure fails closed until the required live streams are
current. Before enabling live trading, verify the account, trading signer,
instruments, prompts, models, schedules, and current account state. Review runs
and positions continuously, especially after provider changes or configuration
edits.

## Workspace

Every agent receives a generated workspace with an OpenCode configuration,
agent instructions, and an MCP adapter. The adapter uses an agent-scoped
HyperVibes API credential. The generated workspace `.env` is backend-owned
state; do not read or modify it. The workspace does not receive the user's
Hyperliquid private key.

The default workspace layout has `AGENTS.md` and `opencode.json` at the root.
The `.opencode/` directory contains agent profiles, commands, and skills.
Analysis files are organized under `scripts/`, while `scripts/user/`, `data/`,
and `scratch/` are writable agent-managed paths.

### Workspace Drift

The agent settings page reports drift when template-managed workspace files no
longer match the current workspace template. Agent-authored files are not part
of this comparison.

When drift is detected, the workspace can be reset with these options:

- **Regular reset** refreshes template-managed files while preserving
  `scripts/user/`, `data/`, and `scratch/`.
- **Hard Reset** deletes the entire workspace, including user-managed and
  agent-generated files, before recreating it from the template.
- **Reset memories** deletes all memories stored for the agent. This option is
  available with Hard Reset.

When changes are made to the default agent workspace template, such as an
application update that changes agent skills or tools, the web interface warns
that the workspace has drifted from the default template.

## Instrument Selection

Instrument selection limits the Hyperliquid perpetual markets an agent can
analyze and trade. At least one instrument must be selected before the agent can
trade. With no instruments selected, the agent does not analyze markets or place
new trades.
