---
slug: /concepts/sub-agents
---

# Sub-agents

Sub-agents are workers that handle specific jobs for an agent. Open Sub-Agents
to configure them and the main conversation.

## Configure a worker

- Start from a template or create a custom worker.
- Set its name, description, instructions, model, and supported thinking level.
- Choose the tools it can use.
- Allow or disable delegation from the main conversation.
- Reuse a persistent conversation or start fresh for each assignment.
- Set an assignment timeout.

The optional default workers are a Technical Analyst, Trader, and Daily Reviewer.
They start paused so you can review their configuration first.

## Run work

- Use Run now to give a worker a task.
- Schedule work after a UTC candle close, with an optional delay.
- Configure event triggers for automatic assignments.
- Choose whether completion only saves the result, notifies the main conversation, or also wakes it to respond.
- Enable or pause workers individually.

## Review and manage work

- Browse run history and open a run's assignment, result, and recorded activity.
- Inspect saved publications and supporting evidence.
- Stop a run or discard a pending inbox request.
- Archive a worker while preserving its history.
- Unarchive a worker when needed. It returns paused.

Pausing or stopping work does not close positions or cancel exchange orders.
All workers share their parent agent's [trading limits](Settings.md).
