---
slug: /concepts/prompts
---

# Instructions

Instructions describe the job you want the main conversation or a sub-agent to do.
Open its configuration under Sub-Agents, edit Instructions, and choose
Save instructions.

## What to include

- The worker's task and the markets it should cover.
- The evidence it should check before reaching a conclusion.
- When it should wait instead of acting.
- What it should save or report when the task ends.

For example, an analyst's instructions might say:

> Review BTC after each hourly candle closes. Check the trend and volume.
> Save a research summary with the evidence time and explain what would change
> your conclusion. Do not place orders.

## Permissions and limits

Instructions do not grant tools or enforce trading restrictions. For a
research-only worker, leave order tools disabled. Set allowed instruments and
exposure limits in [Settings](Settings.md).

Use [Memories](Memory.md) for saved findings and [Insights](Insights.md) for
lessons carried forward. Keep the worker's recurring job in its instructions.
