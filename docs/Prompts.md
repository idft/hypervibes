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

When granted its prompt-update capability, Review may submit an evidence-backed
atomic revision batch for Trading and Analysis. It validates target ownership
and base revisions when submitted, and cannot revise its own prompt.

## Default strategies

The editable Analysis default favors trend continuation and separates directional
bias from entry readiness. It asks for an evidence-backed conclusion for every
selected instrument, explicit entry confirmation and invalidation, confidence
rationale, and at least 1.5:1 estimated reward:risk after costs for actionable
setups. Missing evidence and no-trade conclusions are valid outcomes.

Research uses the system's timeframe-based expiry unless the analyst shortens it
with `metadata.stale_after` or `metadata.valid_for_seconds`. A deadline in prose
does not set memory expiry. Thesis and entry-opportunity lifetimes are distinct;
a memory containing both uses the earlier deadline.

The editable Trading default uses these conservative starting limits:

- Planned loss per setup: **0.25% of current account equity**.
- Aggregate planned loss across positions and pending entries: **1% of equity**.
- Gross notional exposure including pending entries: **1x equity**, without
  netting opposing positions.
- One starter entry using at most half the setup loss budget; additions require
  fresh continuation evidence and share the original budget. No entry ladders
  or averaging down.

Trading evaluates conflicting research, requires protective stops, manages
existing positions even when research is unavailable, and verifies order outcomes
before retrying uncertain submissions. Exits are reduce-only, and protection is
reconciled after partial fills or exits. Planned stop risk is an estimate, not a
guarantee of maximum realized loss.

The editable Review default evaluates decision quality separately from outcomes
for the supplied review window, including partial-day reviews. It assesses
Analysis, Trading, execution, and indicator evidence against the strategy and
information applicable at the time, and records gaps rather than assuming
historical attribution. Relevant indicator inspection is part of every Review
run, even without indicator-writing permission. Historical results are read by
exact run ID where available within the permitted scope; later results cannot
retroactively make a timed-out dependency available to Analysis.

Review applies indicator or Analysis/Trading prompt changes only with the
corresponding capability and material supporting evidence. It favors small
corrections, preserves unrelated operator choices, and distinguishes successful
changes from proposals and failed attempts. A successful indicator run is not
proof of predictive value. Single-day outcomes and tentative hypotheses do not
automatically become strategy changes or durable learnings. Reports cover
evidence coverage, outcomes, research and indicator findings, execution and risk,
changes, learnings, and unresolved questions; no changes warranted is valid.

These are editable agent instructions, not exchange-enforced or server-enforced
risk limits. Default changes apply when initializing new prompts; existing saved
prompt revisions are not rewritten. Operators can customize the strategy and
should keep Analysis and Trading actionability requirements aligned.

## Role boundaries

Analysis uses approved data tools, including the closed-candle market-data
helper, and publishes research memories. Individual prompts and granted
capabilities define the research method and output types; Analysis does not
author reusable code. It can read the server-computed indicator definitions and
results available to its agent, but cannot create or edit indicators.

Trading reads current Analysis context and owns synthesis and execution. It
should write a `trading_decision` for each evaluated instrument when possible,
including no-trade and position-management outcomes, linked to the evidence it
considered.

Scheduled sub-agents receive their prompt as input but cannot modify it. See
[Sub-agents](SubAgents.md) and [Chat](Chat.md).

When granted its indicator-write capability, Review can inspect indicators and
create an evidence-backed definition or immutable version. Indicator source must
not encode trading policy. Trading has no indicator access and continues to
consume Analysis research memories.
