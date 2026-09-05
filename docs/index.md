# Overview

HyperVibes is an open-source, self-hosted application for building and
supervising crypto trading agents on Hyperliquid. It provides the account
boundaries, scheduling, memory, execution gateway, and live operational view
around an agent's LLM and tool runtime.

The OpenCode backend connects agents to any model provider supported by OpenCode,
along with its skills and MCP tools.
This keeps the model and tool runtime flexible while HyperVibes manages the
agent's account, schedules, memory, and exchange access.

HyperVibes is currently tested on Linux only. macOS and Windows have not yet
been tested.

## Start here

- [Install HyperVibes](Installation.md) using containers or from source.
- [Quick start](QuickStart.md) to complete the first setup steps.
- [Configure Hyperliquid](Hyperliquid.md) to understand wallets, signers,
  accounts, fees, and referral settings.
- [Agents](Agents.md) to understand agent setup and controls.
- [Configure AI providers](Providers.md) to connect models through OpenCode.

## Technical Overview

- Rust single binary
- Built-in web UI
- OpenCode backend
- Postgres database
- Deploy with containers

## System Overview

Each agent has its own Hyperliquid account or sub-account. This keeps each
agent's positions, orders, and account activity separate.

Agent sub-agents run automatically. The available sub-agent types are:

- **Analysis** - Runs on a fixed schedule tied to candle closings, such as 15m,
  1h, or 1d. It uses approved data tools for research and publishes the results
  as scoped memory.
- **Trading** - Runs more frequently, defaulting to every 5 minutes. It reads
  published Analysis memory, places orders, and manages positions.
- **Review** - Runs daily to review agent performance and publish accumulated
  learnings.

Custom indicator support is a separate future follow-up and is not currently
implemented.

Prompts for each sub-agent type can be edited to describe your trading strategy.
