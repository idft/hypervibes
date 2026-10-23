---
slug: /concepts/agents
---

# Agents

An agent has a main conversation, a linked Hyperliquid account, and its own
sub-agents and saved work.

## Create an agent

- Give it a name and choose a model.
- Assign your main Hyperliquid account or an available sub-account.
- Create a new sub-account during setup if your Hyperliquid account is eligible.
- Optionally include the default Technical Analyst, Trader, and Daily Reviewer. All start paused.

Each trading account can belong to only one app agent at a time. Complete your
[account approvals](Hyperliquid.md) and connect an [AI provider](Providers.md)
before creating an agent.

## Manage an agent

- Use the agent switcher to move between agents.
- Open its Dashboard for account and worker activity.
- Use Chat to ask questions or request work.
- Configure the main conversation and workers under Sub-Agents.
- Set trading permissions and limits in Settings. New agents start with trading disabled.

## Emergency stop and re-enable

Use **Emergency stop** in the top bar to disable the agent and abort active
chat and worker runs. **Orders and positions remain open** and can still execute
on Hyperliquid. Requests already submitted to the exchange cannot be recalled.

The agent stays disabled across restarts. Use **Enable agent** in the top bar or
[Settings](Settings.md#agent-execution) to resume. Worker and risk settings are
preserved; queued and interrupted work does not replay automatically.

You can still [close positions from the Dashboard](Dashboard.md#close-a-position)
while the agent is disabled.

Deleting an agent removes its configuration and app history. It does not close
positions or cancel exchange orders. See [Settings](Settings.md).
