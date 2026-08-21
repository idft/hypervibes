---
slug: /operation/hyperliquid
---

# Hyperliquid configuration

[Hyperliquid](https://hyperliquid.xyz) is currently the only exchange supported
by HyperVibes because of its worldwide availability and straightforward
integration for AI agents.

## Register with Hyperliquid

Before using HyperVibes, register the wallet you use to sign in with
Hyperliquid:

1. Open Hyperliquid and connect the same wallet you use to sign in to
   HyperVibes. The wallet must be on Arbitrum.
2. Choose **Enable Trading** in Hyperliquid and sign the gas-less transaction.
3. Make the first deposit requested by Hyperliquid, with at least $5 USDC.
4. Return to HyperVibes and refresh the Account page.

See Hyperliquid's [onboarding and trading guide](https://hyperliquid.gitbook.io/hyperliquid-docs/onboarding/how-to-start-trading.md)
for its current wallet connection and deposit instructions. Its [USDC
documentation](https://hyperliquid.gitbook.io/hyperliquid-docs/hypercore/usdc.md)
explains how USDC moves onto HyperCore.

Connecting a wallet alone does not activate a Hyperliquid trading account. The
Account page must show the wallet as registered before you can set up an API
key, approve a builder fee, or create an agent.

HyperVibes currently supports Hyperliquid Unified Accounts. Your main account
and its sub-accounts use the shared Unified USDC balance shown on the Account
page.

## API Keys

Hyperliquid API keys, also called API wallets or agent wallets, are signing
wallets that can place orders on behalf of your main account or its sub-
accounts. They do not hold funds. Your main wallet remains the owner and is
used for login, approvals, and account transfers.

See Hyperliquid's [API wallet documentation](https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/nonces-and-api-wallets.md)
for the exchange-level API wallet model and expiry behavior.

HyperVibes recommends generating the API key from the Account page:

1. Select **Generate API Key**.
2. HyperVibes creates a new signing key and stores it encrypted.
3. Approve the key with your main wallet when Hyperliquid requests approval.

You can instead select **Import existing API key** and provide a key that you
generated manually on Hyperliquid's web site. HyperVibes checks that the key
matches its derived address and stores the private key encrypted. Only import a
key when you understand where it came from and who has had access to it.

API keys expire after six months. Rotate an expiring or expired key from the
Account page by generating a fresh key and approving it with your main wallet.
Do not reuse a key that has expired or been deregistered. HyperVibes uses the
API key for exchange signing; it never gives the key to an agent workspace and
the key does not custody your funds.

## Accounts and sub-accounts

An agent must be assigned exactly one exclusive Hyperliquid trading account.
That account can be your main account or one of your sub-accounts. No two
agents can share the same account. The agent's positions, orders, balances, and
transaction history are kept separate from every other agent's account.

Sub-accounts remain owned by the main account but provide separate trading
boundaries. Hyperliquid allows up to 10 sub-accounts after the main account
reaches $100,000 in trading volume. Every additional $100 million in volume
allows one more sub-account, up to a maximum of 50. Hyperliquid's sub-account
limit is based on the main account's cumulative volume.

See Hyperliquid's [sub-account documentation](https://hyperliquid.gitbook.io/hyperliquid-docs/trading/sub-accounts.md)
for the current volume thresholds and sub-account limits.

After you have an approved API key and enough sub-account capacity, HyperVibes
can create a named sub-account while you create an agent. The application
assigns the new account to that agent after Hyperliquid confirms it. You can
also assign an existing unassigned main account or sub-account from the agent
setup page.

Sub-accounts use the main account's fee tier, but Hyperliquid referral
discounts do not apply to sub-accounts.

## Transactions

HyperVibes imports Hyperliquid account activity for each agent's assigned
account and displays it as a chronological ledger on the agent's Transactions
page. The application combines exchange history with newly received account
events and avoids adding the same event more than once.

The ledger can include:

- trades and fills
- trading fees and realized profit or loss
- funding payments
- deposits, withdrawals, and account transfers
- other Hyperliquid ledger changes

Transactions are historical account activity. Current positions and open orders
are shown separately on the agent page, while the transaction ledger remains
the account's historical record. Hyperliquid's [historical data
documentation](https://hyperliquid.gitbook.io/hyperliquid-docs/historical-data.md)
describes the exchange's own historical-data sources.

## Builder Fee

A builder fee approval is required before HyperVibes can place eligible
perpetual orders. The fee supports ongoing HyperVibes development and is
collected on eligible orders submitted through the application.

Hyperliquid explains builder-code approvals and limits in its [builder code
documentation](https://hyperliquid.gitbook.io/hyperliquid-docs/trading/builder-codes.md).
Its general [fee documentation](https://hyperliquid.gitbook.io/hyperliquid-docs/trading/fees.md)
covers exchange trading fees separately from the HyperVibes builder fee.

The builder fee must be set between **1 basis point (0.01%)** and **10 basis
points (0.10%)**.

Your main wallet signs the approval. You can change or cancel the approval from
the Account page. The selected fee is applied to eligible orders for your
account.

## Referral Discount

The referral discount is optional. Eligible users can apply the HyperVibes
referral code from the Account page for an additional **4% discount on
Hyperliquid fees**.

The main account must have less than $10,000 in cumulative trading volume and
must not already have a referral code. The discount is offered only while the
account remains eligible. Referral discounts do not apply to sub-accounts. See
Hyperliquid's [referral documentation](https://hyperliquid.gitbook.io/hyperliquid-docs/referrals.md)
for its current referral terms.
