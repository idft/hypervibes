# Wallet Security Model

## Roles and Custody

- The **main wallet** is the user's browser wallet. It signs SIWE login,
  builder-fee approval, the one `approveAgent` action for the user's signer,
  and main-account transfers. Its private key is never stored or sent to the
  server.
- Each user has one **trading signer** (Hyperliquid API wallet). Vibetrading
  generates or imports its private key, validates the derived address, and
  stores only AES-256-GCM ciphertext plus the encryption key ID. The signer is
  approved once by the user's main Hyperliquid account, holds no funds, and is
  shared by that user's agents.
- Each agent owns one exclusive **trading account**: either the user's main
  account or one Hyperliquid sub-account. Balances, positions, orders, fills,
  and history are always tracked against this address. The database enforces
  uniqueness by `(trading_account_address, environment)`.
- Agent API keys are unrelated bearer credentials for Vibetrading's API. They
  never grant access to the trading signer.

## Signer Lifecycle

The Wallet page handles signer setup, replacement, and approval. A replacement
must be explicit and must derive a fresh address; the old deregistered address
is never silently reused. Approval timestamps are recorded only after
Hyperliquid accepts the relay. Exchange-provided expiry is stored only when an
authoritative response provides it; it is never synthesized from approval time.

Before every order or cancel action, Vibetrading requires an active agent,
valid owner signer material, recorded approval, and a live signer expiry check
when expiry is available. Any failure blocks all of the owner's exchange
actions with a trading-signer error while preserving account history.

## Server-Side L1 Actions

The server signs `createSubAccount` with the approved user trading signer. It
uses Hyperliquid MessagePack hashing, a big-endian uint64 nonce, the no-vault
marker, and the `Exchange` EIP-712 domain with chain ID `1337`. The browser does
not switch to or configure a fake chain-1337 network and does not sign this
action.

The main wallet continues to sign browser-only actions such as `approveAgent`,
builder-fee approval, and transfers on Arbitrum mainnet. All protected routes
require the session CSRF token and validate the authenticated owner, exact
action fields, recovered signer, and target account.

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
