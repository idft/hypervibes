# Hyperliquid Module

The Hyperliquid subsystem owns market reference data, account-history
journaling, live account state, and the application's order execution gateway.
It runs inside the Vibetrading process. Each enabled agent is supervised as an
account identified by its wallet address and environment. Disabled agents are
not monitored and cannot submit new orders through the gateway.

## Integration Boundaries

`hypersdk` provides mainnet perpetual instruments, live WebSocket state, and
signed order placement and cancellation. Vibetrading owns raw Hyperliquid
`/info` HTTP calls for account history, including fills, funding, non-user
funding ledger updates, and historical orders.

The authenticated `/account` page reads `spotClearinghouseState`, `subAccounts`,
and `userAbstraction` concurrently from `/info` on each load. Its account table
shows the Unified Account's Spot-state USDC `total` in USDC, with unavailable
values shown explicitly rather than using cached or fabricated balances.
Vibetrading currently supports Unified Accounts only. User-signed `sendAsset`
transfers are restricted to the authenticated main account and its currently
discovered sub-accounts, use `spot` for both DEX fields because Unified mode
shares the collateral balance with Spot, and use canonical mainnet USDC.
Hyperliquid account mode is checked again before relay. API-wallet signers
never custody or transfer account funds.

The application, not an agent, owns signing. One encrypted user trading signer
is decrypted only server-side for signed exchange requests and is never written
into an agent workspace or returned through the agent API. Agent accounts are
tracked independently from that signer.

## Monitoring And Journal

At startup the monitor loads and upserts perpetual instrument metadata. It
then supervises one live task for each enabled agent, refreshing that registry
every 30 seconds.

An agent task:

1. Performs startup HTTP synchronization for account history and historical
   orders. Errors are logged; they do not prevent the live loop from starting.
2. Starts the WebSocket loop for account state, fills, user events, and
   clearinghouse state. The loop performs HTTP catch-up on reconnect.
3. Runs an order reconciler every 30 seconds when its exchange clients can be
   initialized.

The durable journal uses typed tables rather than a generic event table:

| Data | Purpose |
| --- | --- |
| `hyperliquid.instruments` | Perpetual market identifiers, precision, leverage, and activity metadata. |
| `hyperliquid.sync_state` | Per-account, per-stream sync checkpoints and status. |
| `hyperliquid.trade_fills` | Exchange fills, prices, sizes, fees, and realized PnL. |
| `hyperliquid.funding_events` | Funding payments and associated market context. |
| `hyperliquid.ledger_events` | Non-funding ledger changes such as transfers and withdrawals returned by the history feed. |
| `hyperliquid.historical_orders` | Historical exchange order state. |
| `hyperliquid.account_timeline` | A view combining fill, funding, and ledger activity in chronological order. |

Journal rows are keyed so overlapping HTTP windows and WebSocket catch-up are
idempotent. The current journal supports only the `live` environment. Testnet
or sandbox support requires a schema and integration change.

Agents can read their own bounded journal windows through the read-only
`vibetrading_list_account_transactions` MCP tool. It returns normalized fill,
funding, and ledger events from `account_timeline`; it never sends an
agent-initiated request to Hyperliquid.

## Operator Stops And Exits

Order placement holds the agent execution lock until the exchange request has
completed. Emergency Stop obtains the same lock before setting the agent
disabled and cancelling the exchange's currently open orders. This prevents a
placement that began immediately before a stop from being missed by the
cancellation pass. Cancellation cannot undo an order that was already filled.

Operator-requested position Close and Close all actions use the server-owned
signer, re-read exchange positions, and submit only reduce-only market orders.
They remain available while the agent is disabled and cannot be invoked through
the agent-facing API.

Live account state is intentionally in memory. It supports the operator UI and
trading dispatch context; the journal remains the durable source for history.

### Live Data Health

The monitor tracks successful clearinghouse, open-orders, and spot-state
snapshots independently. Positions are authoritative only after a fresh
clearinghouse snapshot, open orders only after a fresh open-orders snapshot,
and the unified balance only after both clearinghouse and spot-state snapshots.
Snapshots older than two minutes, or any snapshot while the WebSocket is not
connected, are not authoritative.

The operator UI exposes connection health and a sanitized monitoring error. It never presents an unavailable stream as an
empty positions or orders list. Agent account responses and trading prompts
carry the same health metadata.

New agent-originated exposure fails closed unless fresh clearinghouse and
open-orders snapshots are available from a connected monitor. Pure reduce-only
orders and cancellation remain available so agents and operators can reduce
risk during an outage. Server-authorized operator close actions independently
re-read exchange positions before submitting exits.

## Orders

Agents place and manage orders only through the authenticated `/api/v1/orders`
surface and the workspace MCP adapter. The gateway:

1. Resolves the calling agent from its API key and enforces that agent's
   instrument allowlist.
2. Validates the request and rounds price and size according to the stored
   instrument precision.
3. Generates a client order ID and writes a local pending order before calling
   the exchange.
4. Loads and decrypts the owner's user trading signer and submits or cancels
   against the agent's stored trading account.
5. Records the immediate result and later WebSocket or reconciliation updates.

`hyperliquid.orders` holds the current, agent-attributed order state.
`hyperliquid.order_events` is the append-only audit trail for transitions from
HTTP responses, WebSocket order updates, and reconciliation. Orders can be
linked to memory records and grouped with attached take-profit or stop-loss
legs. The reconciler resolves uncertain submissions and cancels orphaned
reduce-only TP/SL legs after the underlying position is flat.

Market orders use the configured conservative slippage limit before rounding.
The gateway stores both requested and rounded values, which preserves the
execution audit trail.

### Builder Fees

Eligible perp orders submit the per-user builder fee and the server-side
builder address on every batch. The application does not preflight
`maxBuilderFee` before each order or on every account-page request. Instead,
the current remote maximum is looked up lazily on the first account-page
request for a user and builder, then cached in process memory and synchronized
back to the local fee setting. A builder-fee-specific order rejection forces
one targeted refresh. The in-process cache is cleared when the process
restarts, while the database remains synchronized by successful lookups.

The builder address is part of the lookup key and comes from the server-side
constant; Hyperliquid returns only the numeric maximum. A successful signed
approval and a successful order update the local cache without another lookup.
Orders are never automatically retried after a builder-fee rejection; the
original exchange result is retained for the agent and user to resolve.

## Deferred Work

The following remain intentional future work:

- account summary projections or rollups for daily, symbol, fee, funding, and
  realized-PnL reporting
- a product decision on whether to include sub-account and spot transfers in
  the journal scope
- richer multi-account account registry support
- vault activity, staking, deep historical archival, and agent webhooks

The existing journal and order records should be extended when these are
implemented; do not introduce duplicate draft schemas alongside them.
