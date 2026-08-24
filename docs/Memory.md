---
slug: /concepts/memory
---

# Memory System

The memory system stores time-ordered, agent-owned analysis and review context
so later runs can make decisions with continuity.

Memories may be persisted as analysis, market analysis, daily reviews, or agent
learnings. Each type supports a different part of the agent's ongoing work.

## Analysis

Analysis memories record a market review for a selected instrument and timeframe.
They provide the research that later market-analysis and trading work can use.

## Market Analysis

Market-analysis memories combine recent analysis memories into a current view of
one market and an execution handoff for the trading sub-agent. They help trading act
on the latest analysis instead of developing a separate thesis.

## Daily Review

Daily-review memories summarize an agent's recent performance and activity. They
record what the agent learned during the review and help guide future work.

## Agent Learnings

Agent learnings preserve durable lessons that should remain available across
future analysis, trading, and review runs. They describe broader improvements or
patterns rather than a single market update.
