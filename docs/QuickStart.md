---
slug: /quick-start
---

# Quick start

## Run HyperVibes

Run `pnpm install` and `pnpm start` in the source checkout, then open
<http://localhost:3003>. See [Installation](Installation.md) for requirements and
private LAN setup. NPX packages and desktop builds are coming soon.

## Set up your account

1. Sign in with your browser wallet. Use the wallet that owns your Hyperliquid account.
2. Open Account. If Hyperliquid needs account activation, follow the link to complete it first.
3. Generate or import a Hyperliquid API key and approve it with your wallet. Never import your main wallet's private key.
4. Review and approve the builder fee shown on the Account page.
5. Open Providers and connect an AI provider.

[Hyperliquid account](Hyperliquid.md) and [AI providers](Providers.md) explain these steps.

## Create your first agent

1. Choose Create agent and enter a name.
2. Select a model and its thinking level, if supported.
3. Assign your main account or an available sub-account.
4. Keep Include default sub-agents selected if you want a Technical Analyst, Trader, and Daily Reviewer. They start paused.
5. Create the agent and open Chat to ask about its setup.

## Before enabling trading

- Review each sub-agent's instructions, model, tools, and schedule.
- Set allowed instruments and exposure limits in Settings.
- Run an analysis task and inspect its result and saved memories.
- Enable only the sub-agents you want to run automatically.
- Turn on Enable Trading only when you are ready for live orders. It starts off.

Instructions are guidance, not enforced risk limits. Use [Settings](Settings.md)
for trading restrictions. Pausing a worker does not close positions or cancel
orders on Hyperliquid.
