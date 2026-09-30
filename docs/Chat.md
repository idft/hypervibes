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
- `hypervibes_list_analysis_instruments`
- `hypervibes_list_indicators`
- `hypervibes_get_indicator`
- `hypervibes_get_indicator_results`

The header shows each conversation permission's current status. Use **Manage
permissions** to change Orders, Memory, Notifications, Trade notes, Indicators,
or Strategy prompts, then **Save** to apply them. **Cancel** discards your edits.
Permissions can be changed while the conversation is idle.

Each permission supports **Deny**, **Confirm** (one-time approval for each action),
or **Allow**. Configure an agent's starting permissions in **Settings → Chat
permission defaults**, select **Manage permissions**, adjust the permissions in
the modal, then select **Save**. **Cancel** discards your edits. New Web and Telegram
conversations copy these defaults when they are created. Changing the defaults
does not change existing conversations; use **Manage permissions** in a
conversation to change its own permissions.

New agents initially default to Confirm for Orders, Memory, Trade notes,
Indicators, and Strategy prompts, and Deny for Notifications. Existing agents
keep their saved defaults. Read-only tools remain allowed.
The Strategy prompts permission controls `hypervibes_update_strategy_prompt`.

Chat can also create or update an agent-owned indicator using validated
PineScript-subset source. Each mutation follows the Indicators permission and
records the conversation as its provenance. Chat cannot grant
an indicator access to instruments outside the agent's analysis set.
It first lists selected analysis instruments and uses those IDs unchanged. If
none are selected, Chat directs the operator to Settings rather than guessing
instrument IDs or attempting the mutation.

Notifications initially default to Deny. To let Chat send notifications, set
**Notifications** to **Allow** for the conversation, or in the agent's defaults
before creating a new conversation.

## Discuss a prompt

The **Discuss prompt** action on a role page opens Chat with the draft:

1. The user selects a sub-agent prompt target and edits its draft.
2. HyperVibes creates a conversation using the latest Chat model.
3. The draft is included in the opening message when it fits the context limit.
4. Chat can load the saved target prompt when the draft is too long.
5. Any requested MCP update follows the conversation's Strategy prompts permission.

Chat prompt tools only access the current agent's targets. Scheduled sub-agents
receive their prompt revision as input but cannot modify it.
