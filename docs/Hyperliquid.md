# Hyperliquid Module

The Hyperliquid subsystem owns market reference data, account-history
journaling, live account state, and the application's order execution gateway.
It runs inside the Vibetrading process. Each enabled agent is supervised as an
account identified by its wallet address and environment.

## Integration Boundaries

`hypersdk` provides mainnet perpetual instruments, live WebSocket state, and
signed order placement and cancellation. Vibetrading owns raw Hyperliquid
`/info` HTTP calls for account history, including fills, funding, non-user
funding ledger updates, and historical orders.

The application, not an agent, owns signing. An agent's private key is
encrypted at rest, decrypted only server-side for a signed exchange request,
and never written into its workspace or returned through the agent API.

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

Live account state is intentionally in memory. It supports the operator UI and
trading dispatch context; the journal remains the durable source for history.

## Orders

Agents place and manage orders only through the authenticated `/api/v1/orders`
surface and the workspace MCP adapter. The gateway:

1. Resolves the calling agent from its API key and enforces that agent's
   instrument allowlist.
2. Validates the request and rounds price and size according to the stored
   instrument precision.
3. Generates a client order ID and writes a local pending order before calling
   the exchange.
4. Decrypts the server-held agent key and submits or cancels with signed HTTP.
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
