---
slug: /concepts/prompts
---

# Prompts

Prompts define a sub-agent's role. They are separate from its configuration,
model, and per-job Additional Instructions.

## Prompt targets

Every sub-agent has an immutable-revisioned full prompt. An agent has many
Analysis prompt targets and one target for each singleton role.

| Target | Used by |
| --- | --- |
| Analysis | A user-configured research job identified by its sub-agent key. |
| Trading | Research synthesis and order management. |
| Coding | Durable Coding package maintenance. |
| Review | Performance and learning review. |

Prompt targets belong to one agent and are never shared. The global prompt is
an application setting included in sub-agent context. Additional Instructions
remain web-only, are appended to one sub-agent's prompt, and are not part of its
saved revision.

## Editing and revisions

Each role page edits its prompt; an Analysis job detail edits that specific job's
prompt. **Discuss prompt** opens Chat with the current draft as opening context.
Saving creates an immutable revision. Chat changes require one-time OpenCode
confirmation.

Review may submit an evidence-backed atomic revision batch for Trading and only
Analysis jobs that opted in. It snapshots permitted targets and their base
revisions when dispatched, and cannot revise its own or Coding's prompt.

## Role boundaries

The Analysis framework is generic: individual prompts and granted capabilities
define research method and output types. It does not require OHLCV, package
execution, a fixed output type, or one memory per selected instrument.

Trading reads current Analysis context and owns synthesis and execution. It
should write a `trading_decision` for each evaluated instrument when possible,
including no-trade and position-management outcomes, linked to the evidence it
considered.

Scheduled sub-agents receive their prompt as input but cannot modify it. See
[Sub-agents](SubAgents.md) and [Chat](Chat.md).
