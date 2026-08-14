# Wallet Security Model

## Roles and Custody

- The **main wallet** is the user's browser wallet. It signs SIWE login,
  builder-fee approval, the one `approveAgent` action for the user's signer,
  and main-account transfers. Its private key is never stored or sent to the
  server.
- Each user has one **trading signer** (Hyperliquid API wallet). HyperVibes
  generates or imports its private key, validates the derived address, and
  stores only AES-256-GCM ciphertext plus the encryption key ID. The signer is
  approved once by the user's main Hyperliquid account, holds no funds, and is
  shared by that user's agents.
- Each agent owns one exclusive **trading account**: either the user's main
  account or one Hyperliquid sub-account. Balances, positions, orders, fills,
  and history are always tracked against this address. The database enforces
  uniqueness by `(trading_account_address, environment)`.
- Agent API keys are unrelated bearer credentials for HyperVibes's API. They
  never grant access to the trading signer.

## Signer Lifecycle

The Account page handles signer setup, replacement, and approval. A replacement
must be explicit and must derive a fresh address; the old deregistered address
is never silently reused. Approval timestamps are recorded only after
Hyperliquid accepts the relay. Hyperliquid does not expose an API-wallet expiry
lookup, so HyperVibes records the expected six-month signer lifetime from that
approval and treats it as the local expiry deadline. The Account page and
authenticated-page navbar warn when fewer than 30 days remain.

Before every order or cancel action, HyperVibes requires an active agent,
valid owner signer material, recorded approval, and an unexpired local signer
deadline. Any failure blocks all of the owner's exchange actions with a
trading-signer error while preserving account history.

## Agent API Keys

Agent API keys are HyperVibes bearer credentials used by MCP tools and external
agent harnesses to access the application's agent API. They do not grant direct
access to the trading signer. They are deliberately stored in plaintext and
rendered on the agent Settings page so the owner can retrieve and copy them
later; they are not rotated or revoked. This is an intentional usability
tradeoff for this low-scope integration credential.

## Server-Side L1 Actions

The server signs `createSubAccount` with the approved user trading signer. It
uses Hyperliquid MessagePack hashing, a big-endian uint64 nonce, the no-vault
marker, and the `Exchange` EIP-712 domain with chain ID `1337`. The browser does
not switch to or configure a fake chain-1337 network and does not sign this
action.

The main wallet continues to sign browser-only actions such as `approveAgent`,
builder-fee approval, and `sendAsset` transfers on Arbitrum mainnet. The
`/account` page displays fresh Hyperliquid Unified Account Spot USDC totals for
the main account and every discovered owned sub-account. HyperVibes
currently supports Unified Accounts only. Transfers use Hyperliquid's `spot`
route for the shared Unified USDC balance, are limited to those accounts and
canonical USDC, and are rejected when the account mode cannot be verified or
is not Unified. The server re-fetches ownership and validates the recovered
signer before relay. All protected routes
require the session CSRF token and validate the authenticated owner, exact
action fields, recovered signer, and target account.

The API-wallet signer never custodies or transfers funds. It is used for
agent trading and server-signed sub-account creation only; account transfers
are always user-main-wallet signatures.

## Agent Bootstrap

New agents require an approved user trading signer before creation. The Create
Agent page can create a new sub-account immediately: the server signs
`createSubAccount` with the user signer, relays it, and reconciles the exact
named account, then the refreshed account list selects it. The agent is saved
with `lifecycle = 'active'`, its OpenCode workspace is generated, its default
strategy prompts are inserted, and the default OpenCode job schedules and
market-analysis hook are seeded. The user is redirected straight to the agent
page. Exchange rejection is preserved and a successful-but-unreconciled
creation is reconciled before another attempt.
