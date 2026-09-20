---
slug: /concepts/agents
---

# Agents

An agent is an isolated configuration with one assigned trading account,
separate analysis and trading instrument sets, sub-agent prompts, and scheduled
sub-agents.

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
learnings and can revise Trading and Analysis prompts when granted that
capability. Missing or stale research does not block the scheduler or order
gateway.

The interface also shows positions, orders, transactions, memories,
conversations, and Indicators. Missing or stale live account data is marked
unavailable; new agent-originated exposure fails closed until required live
streams are current.

## Reset memories

The agent settings page provides a synchronous **Reset memories** action that
deletes all memories for the agent after confirmation. It does not change
sub-agent prompts or account settings.

## Instrument selection

Analysis instruments limit the markets Analysis and indicators can inspect.
Trading instruments are the strict allowlist for new order exposure. Adding an
Analysis instrument never automatically adds it to the Trading allowlist. An
Analysis run explicitly granted the Trading-instrument capability may promote an
active Analysis instrument or remove a Trading instrument. Removing the final
instrument pauses new exposure without closing positions. At least one Trading
instrument must be selected before the agent can trade.

## Indicators

The Indicators tab manages agent-owned, versioned PineScript-subset programs.
Definitions target an explicit subset of the current analysis instruments and
run only on server-fetched closed Hyperliquid candles. The tab shows recent
status and values, supports PineScript text import, and charts the latest
successful run. Editing creates a new immutable version rather than changing
historical source or output. Loop statements are not part of the supported
subset because the embedded Pine runtime cannot enforce an instruction budget.
