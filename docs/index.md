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

Agent jobs run automatically. The available job types are:

- **Analysis** - Runs on a fixed schedule tied to candle closings, such as 5m,
  15m, or 1h. It executes strategy code written by the coding agent and saves
  the results as memory.
- **Market analysis** - Compiles an overall analysis of one market from
  previously saved memories. It runs after each batch of analysis jobs.
- **Trading** - Runs more frequently, defaulting to every 5 minutes. It
  reviews the current market-analysis state, places orders, manages positions,
  and manages stop-loss and take-profit orders.
- **Daily review** - Reviews agent performance and updates memory with
  accumulated learnings.
- **Coding** - Writes Python code for technical analysis and custom
  indicators.

Prompts for each job type can be edited to describe your trading strategy.
