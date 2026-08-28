---
slug: /concepts/chat
---

# Chat

The Chat tab provides a persistent conversation with one agent through the
shared OpenCode backend. A conversation is mapped to a dedicated OpenCode
session and is independent of scheduled sub-agent runs.

Each generated workspace registers a local stdio HyperVibes MCP server with
OpenCode. The adapter reads workspace-scoped HyperVibes credentials and calls
the authenticated agent API; it never receives a Hyperliquid private key.

## Start a conversation

Choose a provider, model, and optional thinking mode when creating a
conversation. Later turns keep that conversation's model selection, while
OpenCode records model attribution for each message. The transcript mirrors
assistant messages, tool activity, errors, token and context usage, cost, and
compaction telemetry through live updates.

Conversation compaction and deletion are OpenCode session operations and are
available only while the session is idle. A conversation belongs to the agent
that created it; deleting the agent removes its idle mapped sessions and
agent-owned conversation records.

## Tools available in Chat

Chat uses the `agent-conversations` OpenCode profile. It has no native shell,
filesystem, web, or task tools and never reads the workspace `.env` file.
HyperVibes MCP tools are scoped to the current agent.

The profile is defined in the container-global OpenCode configuration and
duplicated in the workspace template. Existing workspaces use the updated
profile after the OpenCode image is rolled out; they do not need regeneration
just for this profile change.

The following read tools are allowed by default:

- `hypervibes_get_account`
- `hypervibes_list_strategy_prompts`
- `hypervibes_get_strategy_prompt`
- `hypervibes_get_latest_analysis`
- `hypervibes_get_market_analysis`
- `hypervibes_get_memory_detail`
- `hypervibes_list_memories`
- `hypervibes_list_orders`
- `hypervibes_list_account_transactions`
- `hypervibes_get_order`

The following tools are available subject to the conversation's permissions:

- `hypervibes_submit_orders`
- `hypervibes_cancel_orders`
- `hypervibes_cancel_all_orders`
- `hypervibes_write_memory`

Order and memory-write permissions are configured as **Deny**, **Confirm**, or
**Allow**. Confirm asks the user for a one-time OpenCode approval before the
tool call continues. Updating a strategy prompt always requires confirmation
through `hypervibes_update_strategy_prompt`.

Each conversation now persists a notification policy that defaults to **Deny**.
The Chat profile continues to deny notification sending while conversation
workspace routing and per-session approval scoping are implemented; the stored
policy is intentionally not rendered into OpenCode permissions until then.

## Discuss a prompt

The Prompts tab's **Discuss prompt** action is a shortcut into Chat:

1. The user selects a prompt kind and edits a draft.
2. HyperVibes creates a new conversation using the latest Chat model.
3. The draft is included in the opening message when it fits the conversation
   limit.
4. The user and Chat can review the draft, while Chat can load the saved prompt
   with `hypervibes_get_strategy_prompt` when the draft was too long.
5. A requested MCP update waits for the user's one-time confirmation.

Chat prompt tools can only access the current agent's prompts. Scheduled sub-agents
receive their prompts as input but cannot modify them.

See [Prompts](Prompts.md) for prompt roles and [AI Providers](Providers.md) for
provider and model configuration.
