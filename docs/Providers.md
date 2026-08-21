---
slug: /providers
---

# AI Providers

HyperVibes uses one shared OpenCode service as its agent execution backend.
Every AI provider supported by [OpenCode](https://opencode.ai/) is supported by
HyperVibes. See [Models.dev](https://models.dev/) for provider and model
information. OpenCode runs the LLM and tool calls inside each generated agent
workspace.

## Connecting a Provider

Manage API-key and OAuth provider connections from the Provider connections
page. Provider credentials are stored by OpenCode in its retained data volume,
not on individual agents and not in the HyperVibes database.

Provider configuration is global to the shared OpenCode service. Once a
provider is connected, select an available model for each scheduled job or
conversation. Some models also provide a selectable thinking mode.

AI providers may authenticate with API keys, OAuth, or both. Connect and verify
providers from the Provider connections page.

After a provider is connected, disconnected, or completes OAuth, HyperVibes
queues a provider configuration reload. The reload waits until OpenCode
sessions are idle because it can interrupt in-flight inference. Once idle,
HyperVibes refreshes the shared OpenCode provider cache from retained auth
storage; existing sessions remain, but an in-flight inference may be
interrupted.
