---
slug: /development/architecture
---

# How agents work

Each agent combines a main conversation with specialist sub-agents.

- The main conversation is where you ask questions and request work. It can delegate tasks to enabled sub-agents.
- Sub-agents handle tasks such as research, trading, and performance reviews. You choose their instructions, models, tools, and schedules.
- Memories save individual research reports, decisions, and reviews.
- Insights hold lessons for the agent or a particular sub-agent.
- The linked Hyperliquid account holds the agent's actual positions and orders.

An agent's tool access determines what it can do. Your risk settings restrict
new trading exposure across the agent and its workers. Written instructions do
not replace these controls.

The app must stay running for scheduled work. Stopping the app or pausing
workers does not remove orders or positions already on Hyperliquid.

Start with [Agents](Agents.md), then review [Sub-agents](SubAgents.md) and
[Settings](Settings.md).
