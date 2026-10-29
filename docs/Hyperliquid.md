---
slug: /operation/hyperliquid
---

# Hyperliquid

Open Account to manage the wallet approvals HyperVibes needs for trading.
Sign in with the wallet that owns your Hyperliquid account.

## Activate your account

If the page asks you to enable trading, open Hyperliquid with the same wallet.
Complete its activation and deposit steps, then return and refresh Account.

## Trading API key

- Generate a trading API key and approve it with your browser wallet.
- Import an existing Hyperliquid trading API key if you already have one.
- Check its approval and expiry status.
- Re-generate it when you need a replacement.

The import field is for the trading API key's private key, never your main
wallet's private key or seed phrase. Protect this key because it authorizes
trading on your account.

## Builder fee

- Review the builder fee shown in the app before approving it.
- Approve or change it with your wallet.
- Cancel the approval from Account if needed.

The builder fee applies to eligible HyperVibes orders in addition to exchange
fees. An approval is not a deposit or a subscription payment.

## Trading accounts

- View your main account and sub-accounts with their balances.
- Check which app agent uses each account.
- Assign an unused account when creating an agent.
- Create a sub-account during agent setup if Hyperliquid permits it.

Each account can belong to only one app agent at a time. Use Hyperliquid to
manage deposits and transfers. The agent order tools trade main-DEX perpetuals;
they do not provide general spot trading or fund transfers.

## Stopping activity and closing positions

[Emergency stop](Agents.md#emergency-stop-and-re-enable) disables the app agent
and aborts its runs without changing exchange orders or positions. Existing
orders can still fill while the agent is disabled.

For a manual exit, use **Close at market** beside the position on the
[Dashboard](Dashboard.md#close-a-position). This cancels that market's orders and
attempts a reduce-only close, even while the agent is disabled. Verify the
result: guarded market orders may only partially fill, and network failures can
leave outcomes unknown.

App [risk settings](Settings.md) do not restrict trades you place directly on
Hyperliquid or through other apps.
