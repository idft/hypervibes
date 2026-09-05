---
slug: /concepts/prompts
---

# Prompts

Prompts define a sub-agent's role. They are separate from its configuration
and model, and each sub-agent owns its complete prompt.

## Prompt targets

Every sub-agent has an immutable-revisioned full prompt. An agent has many
Analysis prompt targets and one target for each singleton role.

| Target | Used by |
| --- | --- |
| Analysis | A user-configured research job identified by its sub-agent key. |
| Trading | Research synthesis and order management. |
| Review | Performance and learning review. |

Prompt targets belong to one agent and are never shared. The global prompt is
an application setting included in sub-agent context.

## Editing and revisions

Each role page edits its prompt; an Analysis sub-agent detail edits that
specific sub-agent's prompt. New Analysis sub-agents start with the default
Analysis prompt, which can be customized during creation. **Discuss prompt**
opens Chat with the current draft as opening context. Saving creates an
immutable revision. Chat changes require one-time OpenCode confirmation.

Review may submit an evidence-backed atomic revision batch for Trading and only
Analysis jobs that opted in. It snapshots permitted targets and their base
revisions when dispatched, and cannot revise its own prompt.

## Role boundaries

Analysis uses approved data tools, including the closed-candle market-data
helper, and publishes research memories. Individual prompts and granted
capabilities define the research method and output types; Analysis does not
author reusable code. Custom indicators are a separate future follow-up and are
not currently implemented.

Trading reads current Analysis context and owns synthesis and execution. It
should write a `trading_decision` for each evaluated instrument when possible,
including no-trade and position-management outcomes, linked to the evidence it
considered.

Scheduled sub-agents receive their prompt as input but cannot modify it. See
[Sub-agents](SubAgents.md) and [Chat](Chat.md).
