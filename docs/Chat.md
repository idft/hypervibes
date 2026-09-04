---
slug: /concepts/chat
---

# Chat

The Chat tab is a persistent OpenCode conversation for one agent. It maps to a
dedicated OpenCode session and is independent of scheduled sub-agent runs.
Workspaces register a local stdio HyperVibes MCP server that uses workspace-
scoped agent credentials and never receives a Hyperliquid private key.

## Start a conversation

Choose a provider, model, and optional thinking mode at creation. Later turns
retain that selection while OpenCode records per-message attribution. The
transcript includes messages, tool activity, errors, token/context usage, cost,
and compaction telemetry through live updates.

Compaction and deletion are OpenCode session operations available only while the
session is idle. Deleting an agent removes its idle mapped sessions and
agent-owned conversation records.

The `agent-conversations` profile has no native shell, filesystem, web, or task
tools and never reads the workspace `.env`. Its default read tools are:

- `hypervibes_get_account`
- `hypervibes_list_strategy_prompts`
- `hypervibes_get_strategy_prompt`
- `hypervibes_get_trading_context`
- `hypervibes_get_memory_detail`
- `hypervibes_list_memories`
- `hypervibes_list_orders`
- `hypervibes_list_account_transactions`
- `hypervibes_get_order`

Order actions and memory writes use the conversation's Deny, Confirm, or Allow
permission. Prompt updates always require one-time confirmation through
`hypervibes_update_strategy_prompt`.

Conversation notification policy defaults to Deny. Chat continues to deny
notification sending while conversation workspace routing and per-session
approval scoping are implemented.

## Discuss a prompt

The **Discuss prompt** action on a role page opens Chat with the draft:

1. The user selects a sub-agent prompt target and edits its draft.
2. HyperVibes creates a conversation using the latest Chat model.
3. The draft is included in the opening message when it fits the context limit.
4. Chat can load the saved target prompt when the draft is too long.
5. Any requested MCP update waits for one-time confirmation.

Chat prompt tools only access the current agent's targets. Scheduled sub-agents
receive their prompt revision as input but cannot modify it.
