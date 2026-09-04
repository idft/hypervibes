---
slug: /concepts/agents
---

# Agents

An agent is an isolated configuration with one assigned trading account,
selected perpetual instruments, sub-agent prompts, a durable Coding package,
and scheduled sub-agents.

## Configure and operate an agent

Scheduled trading requires an active enabled agent with an assigned account, at
least one selected perpetual instrument, at least one enabled Analysis job, an
enabled Trading job, and an explicit model for every enabled scheduled job.
HyperVibes otherwise reports `Paused` or `Setup required`.

The agent navigation has Analysis, Trading, Coding, and Review pages beneath a
non-clickable Sub-agents heading. Analysis is a multi-instance management page;
Trading, Coding, and Review are singleton configuration pages. There is no
market-analysis role or standalone Prompts page.

Analysis publishes scoped research memories. Trading synthesizes the current
research context, records `trading_decision` audit memories when possible, and
manages orders. Review records outcomes and learnings and can revise Trading and
opted-in Analysis prompts. Missing or stale research does not block the
scheduler or order gateway.

The interface also shows positions, orders, transactions, memories, and
conversations. Missing or stale live account data is marked unavailable; new
agent-originated exposure fails closed until required live streams are current.

## Coding package

An agent's only durable filesystem state is its Coding package at
`packages/<agent-key>/` under the workspace root. It contains `manifest.json`
and Coding-defined files, and is created by the first successful Coding
promotion. Runs, Coding candidates, and conversations use isolated workspaces.
Analysis can inspect and execute the read-only package copy; only Coding can
change the durable package.

The Coding page configures its singleton job and displays package status, a safe
file browser and previews, and Coding task status including failure and report
summaries. It never lists or reads `.env` files.

## Reset memories

The agent settings page provides a synchronous **Reset memories** action that
deletes all memories for the agent after confirmation. It does not change
package files or queue a maintenance task.

## Instrument selection

Selected instruments limit markets an agent can analyze and trade. At least one
must be selected before the agent can trade. Analysis and Trading skip market
work when none are selected; Coding and Review do not.
