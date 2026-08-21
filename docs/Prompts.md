---
slug: /concepts/prompts
---

# Prompts

Prompts describe an agent's trading strategy and the role of each job. They are
separate from job configuration, model selection, and per-job Additional
Instructions.

## Strategy prompts

Each agent has one saved prompt for each of these kinds:

| Prompt | Used by |
| --- | --- |
| `analysis` | Scheduled market analysis at a selected timeframe. |
| `market_analysis` | The follow-up that turns analysis memories into an execution handoff. |
| `trading` | The job that acts on the current market-analysis handoff. |
| `daily_review` | The performance and learning review. |
| `analysis_coding` | The request-gated job that maintains `scripts/user/`. |

Each prompt belongs to one agent and is not shared between agents.

The global prompt is separate. It is an application setting included in job
context, while a strategy prompt describes one agent's strategy role. A job's
Additional Instructions are also separate: they apply only to that job and are
editable in the web interface.

## Edit a prompt

The Prompts tab lets a user edit and save each strategy prompt. It identifies
the job role and provides **Discuss prompt**, which starts a Chat conversation
with the current draft as its opening context.

The discussion uses the most recent Chat model. If the agent has no previous
Chat model, select one before the conversation is created. Saving a prompt from
the editor writes it directly; requesting an update from Chat requires a
one-time OpenCode confirmation.

## Prompt boundaries

Scheduled jobs use their selected prompt but cannot edit it. Chat sessions can
read and update prompts belonging to their own agent. Additional Instructions
remain editable only in the web interface and apply only to the job where they
are entered.

Trading normally consumes the latest `market_analysis` memory rather than
developing a new thesis. A conditional market-analysis memory can authorize a
finite confirmation check, but trading cannot change that memory's thesis,
levels, indicators, or execution plan.

See [Jobs](Jobs.md) for scheduling and [Chat](Chat.md) for prompt discussions.
