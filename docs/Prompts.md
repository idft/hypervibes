---
slug: /concepts/prompts
---

# Prompts

Prompts describe an agent's trading strategy and the role of each sub-agent. They are
separate from sub-agent configuration, model selection, and per-sub-agent Additional
Instructions.

## Strategy prompts

Each agent has one saved prompt for each of these kinds:

| Prompt | Used by |
| --- | --- |
| `analysis` | Scheduled market analysis at a selected timeframe. |
| `market_analysis` | The follow-up that turns analysis memories into an execution handoff. |
| `trading` | The sub-agent that acts on the current market-analysis handoff. |
| `daily_review` | The performance and learning review. |
| `analysis_coding` | The request-gated sub-agent that maintains the durable Coding package. |

Each prompt belongs to one agent and is not shared between agents.

The global prompt is separate. It is an application setting included in sub-agent
context, while a strategy prompt describes one agent's strategy role. A sub-agent's
Additional Instructions are also separate: they apply only to that sub-agent and are
editable in the web interface.

## Edit a prompt

The Prompts tab lets a user edit and save each strategy prompt. It identifies
the sub-agent role and provides **Discuss prompt**, which starts a Chat conversation
with the current draft as its opening context.

The discussion uses the most recent Chat model. If the agent has no previous
Chat model, select one before the conversation is created. Saving a prompt from
the editor creates an immutable revision; requesting an update from Chat requires
a one-time OpenCode confirmation.

Prompt changes still create immutable revisions for auditability. Revision history
and rollback are not displayed on the Prompts tab. Daily review may automatically
submit an evidence-backed, atomic revision batch for `analysis`, `market_analysis`,
and `trading` when the per-agent capability is enabled. It cannot revise its own or
the analysis-coding prompt.

## Prompt boundaries

Scheduled sub-agents use their selected prompt but cannot edit it. Chat sessions can
read and update prompts belonging to their own agent. Additional Instructions
remain editable only in the web interface and apply only to the sub-agent where they
are entered.

Trading consumes the latest `market_analysis` memory rather than developing a
new thesis. Trading cannot change that memory's thesis, levels, indicators, or
execution plan, and it bases order and position management on memories,
account and order state, and its approved HyperVibes MCP tools only.

See [Sub-agents](SubAgents.md) for scheduling and [Chat](Chat.md) for prompt discussions.
