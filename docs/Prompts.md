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
immutable revision. Chat changes follow the conversation's Strategy prompts
permission, which defaults to one-time OpenCode confirmation.

When granted its prompt-update capability, Review may submit an evidence-backed
atomic revision batch for Trading and Analysis. It validates target ownership
and base revisions when submitted, and cannot revise its own prompt.

## Default strategies

The editable Analysis default favors trend continuation and separates directional
bias from entry readiness. It asks for an evidence-backed conclusion for every
selected instrument, explicit entry confirmation and invalidation, confidence
rationale, and at least 1.5:1 estimated reward:risk after costs for actionable
setups. Missing evidence and no-trade conclusions are valid outcomes.

New Analysis research expiry is exclusively backend-controlled: two cycles of
the immutable producing run's schedule from its evidence boundary, or 30 minutes
after publication for unscheduled runs. Analysis cannot override it with
`metadata.stale_after` or `metadata.valid_for_seconds`; caller values are silently
ignored. MCP tool descriptions and Analysis runtime/default instructions no longer
ask the analyst to choose a deadline. Older customized prompts can still supply
expiry metadata, but cannot change the backend deadline. A deadline in prose does
not set memory expiry. Distinguish directional context from entry readiness and specify
price/event invalidation; fresh research does not make an invalidated or chased
entry actionable.

Runtime instructions prefer compact version-1 handoffs separating quoted source
observations from interpretation, with exact run/bar references and invalidation.
Analysis performs a pre-publication consistency check of IDs, source rows,
candle open/derived close times, visual offsets, and EMA ordering. Partial or
unavailable evidence must be labeled rather than reconstructed from later runs.
The backend does not verify these research claims. Legacy prose handoffs remain
compatible. Corrections link to exact originals using `corrects`; durable
data-quality guidance is published separately from expiring entry details.

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

Runtime Trading instructions require successful decision IDs for opening orders
and preserve reduce-only protection if logging fails. IDs are copied unchanged
from returned context. A definitive indexed invalid-link 422 allows one context
refetch, reference repair, and write retry; ambiguous failures are reported without
blind retries or order replay. Corrections apply only to their structured targets,
and superseded entry corrections do not become fresh theses.

The editable Review default evaluates decision quality separately from outcomes
for the supplied review window, including partial-day reviews. It assesses
Analysis, Trading, execution, and indicator evidence against the strategy and
information applicable at the time, and records gaps rather than assuming
historical attribution. Relevant indicator inspection is part of every Review
run, even without indicator-writing permission. Historical results are read by
exact run ID where available within the permitted scope; later results cannot
retroactively make a timed-out dependency available to Analysis.

Review independently samples a bounded representative set of publications,
including runs that did not self-correct, comparing claims to exact frozen
numeric outputs/source rows. It records sample IDs, coverage and limitations,
counts self-corrections separately from sampled factual findings, and assesses
decision/expiry/link coverage independently of trade outcomes. This is an
assessment workflow, not an automatic research-acceptance validator.

Review applies indicator or Analysis/Trading prompt changes only with the
corresponding capability and material supporting evidence. It favors small
corrections, preserves unrelated operator choices, and distinguishes successful
changes from proposals and failed attempts. A successful indicator run is not
proof of predictive value. Single-day outcomes and tentative hypotheses do not
automatically become strategy changes or durable learnings. Reports cover
evidence coverage, outcomes, research and indicator findings, execution and risk,
changes, learnings, and unresolved questions; no changes warranted is valid.
The default Review prompt also sends one concise notification summarizing the
review window, findings, changes, learnings, and outstanding issues after writing
the review memory. Manual partial-day reviews are labeled interim.

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
