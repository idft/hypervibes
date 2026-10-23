---
slug: /concepts/chat
---

# Chat

Chat is the agent's main conversation. Use it to ask about markets, review
account activity, or request work from sub-agents.

## Conversation controls

- Send a message or queue one while the agent is busy.
- Follow replies and tool activity as they arrive.
- Load earlier messages, including messages from before a context reset.
- Change the model and supported thinking level when the conversation is idle.
- Check context usage and total conversation usage.
- Stop the active response.

**Stop response** only interrupts the conversation; it does not disable the
agent or its workers. Use the top-bar [Emergency stop](Agents.md#emergency-stop-and-re-enable)
to disable all agent execution. While disabled, history stays readable but new
chat messages, context operations, and worker execution are unavailable until
you explicitly enable the agent again.

## Manage context

- Use Compact to summarize the conversation and reduce the context sent to the model.
- Use Reset context to start fresh without deleting the visible conversation history.

Resetting context does not delete saved memories or insights. Stopping a response
does not undo tool actions or exchange orders that have already completed.

## What the agent can do

The main conversation can only use its enabled tools. Configure it under
Sub-Agents. Delegation also depends on the worker's permissions and availability.

If you enable order tools and trading, a chat request can lead to real orders.
Use [Settings](Settings.md) for enforced trading limits, not chat instructions alone.

You can also use the same conversation through the [Telegram bridge](Telegram.md).
