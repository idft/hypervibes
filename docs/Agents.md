---
slug: /concepts/agents
---

# Agents

An agent is an isolated configuration with one assigned trading account,
selected perpetual instruments, sub-agent prompts, and scheduled sub-agents.

## Configure and operate an agent

Scheduled trading requires an active enabled agent with an assigned account, at
least one selected perpetual instrument, at least one enabled Analysis job, an
enabled Trading job, and an explicit model for every enabled scheduled job.
HyperVibes otherwise reports `Paused` or `Setup required`.

The agent navigation has Analysis, Trading, and Review pages beneath a
non-clickable Sub-agents heading. Analysis is a multi-instance management page;
Trading and Review are singleton configuration pages. There is no separate
market-analysis role or standalone Prompts page.

Analysis uses approved data tools and publishes scoped research memories.
Trading synthesizes the current research context, records `trading_decision`
audit memories when possible, and manages orders. Review records outcomes and
learnings and can revise Trading and opted-in Analysis prompts. Missing or stale
research does not block the scheduler or order gateway.

The interface also shows positions, orders, transactions, memories, and
conversations. Missing or stale live account data is marked unavailable; new
agent-originated exposure fails closed until required live streams are current.

## Reset memories

The agent settings page provides a synchronous **Reset memories** action that
deletes all memories for the agent after confirmation. It does not change
sub-agent prompts or account settings.

## Instrument selection

Selected instruments limit markets an agent can analyze and trade. At least one
must be selected before the agent can trade. Analysis and Trading skip market
work when none are selected; Review does not.
