# Hermes Agent Plugin

WARNING: THIS IS A WORK IN PROGRESS AND IS IDEAS ONLY.
NOTHING IN THIS FILE SHOULD BE IMPLEMENTED YET.

Related docs:

- `README.md` for top-level V2 direction
- `Agents.md` for agent registry and API key identity
- `Memory.md` for the memory subsystem the plugin will integrate with
- `Hyperliquid.md` for the execution gateway design

## Goal

Design the Hermes plugin that V2 agents will use to trade through the
Vibetrading backend.

The plugin lives in this repository and is installed into a Hermes agent
instance. It exposes tools and bundled skills that let an agent:

- fetch market data directly from Hyperliquid
- run local technical analysis
- read and write structured memory through the Vibetrading backend
- place orders through the Vibetrading execution gateway
- inspect account status

## Plugin Location

The plugin source should live under this repository, not a separate repo.

Proposed directory:

```text
hermes-plugin/vibetrading/
```

This keeps the plugin versioned with the backend. How it gets installed into
Hermes is still open — possibilities include manual copy, a Hermes skills tap,
a future install API, or packaging. Keeping everything in one repo is the
current priority.

## Plugin Structure

A Hermes plugin is a Python package with a manifest and registration hook.

```text
hermes-plugin/vibetrading/
├── plugin.yaml           # manifest: name, version, tools, hooks, requires_env
├── __init__.py           # register(ctx): wires schemas, handlers, skills, hooks
├── schemas.py            # tool descriptions / JSON schemas for the LLM
├── tools.py              # tool handler implementations
├── requirements.txt      # Python dependencies for the plugin
└── skills/
    └── trading-workflow/
        └── SKILL.md      # instructions for the agent decision loop
```

See the Hermes plugin guide for the full contract:
https://hermes-agent.nousresearch.com/docs/guides/build-a-hermes-plugin

## Environment Variables

The plugin declares these in `plugin.yaml` under `requires_env`:

| Variable | Purpose |
|----------|---------|
| `VIBETRADING_BASE_URL` | Base URL of the Vibetrading backend (e.g., `http://localhost:3000`) |
| `VIBETRADING_API_KEY` | Per-agent API key from `agents.registry.api_key`. Authenticates the agent to the backend. |
| `HYPERLIQUID_ENVIRONMENT` | `mainnet` or `testnet` |
| `HYPERLIQUID_ADDRESS` | The agent's Hyperliquid wallet / account address. Used for direct Hyperliquid reads. |

Hermes will prompt for missing variables during install and save them to `.env`.

## Tools

### `fetch_candles`

Retrieves OHLCV candlestick data **directly from Hyperliquid** using the
official Hyperliquid Python SDK.

Inputs:

- `instrument` — instrument identifier (v1 will likely hardcode BTC/USD until the instrument model is finalized)
- `timeframe` — candle interval; left to the model's choice (e.g., `1m`, `5m`, `15m`, `1h`, `4h`, `1d`)
- `limit` or time range — how many candles or start/end timestamps

Output:

- JSON array of candles: `[[timestamp, open, high, low, close, volume], ...]` or similar SDK-native shape

Notes:

- The plugin, not the backend, owns this call.
- Rate limits and caching are deferred.

### `analyze_market`

Runs local Python analysis on candle data returned by `fetch_candles`.

Inputs:

- `candles` — candle payload from `fetch_candles`
- `indicators` — optional list of requested calculations (SMA, EMA, RSI, MACD, support/resistance levels, etc.)

Output:

- JSON summary of computed indicators and a short narrative

Notes:

- The skill writes its own Python analysis code.
- No backend endpoint for analysis.
- Keep dependencies lightweight; add required packages to `requirements.txt`.

### `read_memory` (stub)

Reads structured memory records from the Vibetrading backend.

Inputs (planned):

- `agent_key` or resolved from API key
- `symbol`
- `timeframe`
- `memory_type`
- time range or `active_only`

Output:

- JSON list of memory records or active state

Notes:

- The backend memory subsystem is not implemented yet.
- This tool is a stub with a defined contract so it can be wired up later.

### `write_memory` (stub)

Writes structured memory records to the Vibetrading backend.

Inputs (planned):

- `memory_type` — `observation`, `hypothesis`, `plan`, `outcome`, `reflection`
- `symbol`, `timeframe`, `summary`, `thesis`, `confidence`, etc.
- `parent_memory_ids` for lineage

Output:

- JSON with created record IDs

Notes:

- The backend memory subsystem is not implemented yet.
- This tool is a stub with a defined contract.

### `place_order`

Submits an order through the Vibetrading execution gateway, **not** directly
to Hyperliquid.

Inputs:

- `instrument`
- `side` — `buy` or `sell`
- `size`
- `order_type` — `market`, `limit`, etc.
- `price` — for limit orders
- `time_in_force`
- `reduce_only`
- `memory_record_ids` — references to the memory records that justified the decision

Output:

- JSON with intent/submission status and tracking IDs

Notes:

- The backend gateway records execution intent before forwarding to Hyperliquid.
- Linking the order to source memory is a core idea; pass `memory_record_ids` when available.
- Execution gateway is not implemented yet; this is a stub contract.

### `get_account_status`

Fetches current Hyperliquid account status directly from Hyperliquid using the
agent's `HYPERLIQUID_ADDRESS`.

Inputs:

- `HYPERLIQUID_ADDRESS` from environment
- `HYPERLIQUID_ENVIRONMENT` from environment

Output:

- JSON with available balance, open positions, margin, recent fills, etc.

Notes:

- This tool reads from Hyperliquid directly, not the backend journal.
- It gives the agent a live snapshot for decision-making.

## Bundled Skills

### `vibetrading:trading-workflow`

A `SKILL.md` that teaches the agent how to use the tools together. The basic
loop:

1. `get_account_status` — know current risk / position state
2. `fetch_candles` — get market data for the chosen instrument and timeframe
3. `analyze_market` — compute indicators and form a view
4. `read_memory` — load higher-timeframe context and prior plans
5. Decide whether to trade
6. If trading: `place_order` with linked `memory_record_ids`
7. `write_memory` — record the decision, plan, or outcome

The exact prompt text should live in `skills/trading-workflow/SKILL.md`.

### `vibetrading:memory-analysis` (future)

A planned skill or cron-driven workflow that:

- reads past memory records
- compares predictions against actual outcomes
- scores analysis quality
- suggests prompt or policy improvements

This is part of the plugin's responsibilities but is intentionally deferred
until the memory subsystem exists.

## Hooks

Possible plugin hooks:

- `post_tool_call` — audit log of every Vibetrading tool call
- `pre_llm_call` — optionally inject recent active memory context before each turn

Both are optional for v1.

## Backend API Contracts (planned)

The plugin will call these backend endpoints once they exist:

| Endpoint | Purpose |
|----------|---------|
| `GET /api/v1/memory/records` | `read_memory` |
| `POST /api/v1/memory/records` | `write_memory` |
| `POST /api/v1/orders` | `place_order` |

All agent-facing endpoints authenticate via the `VIBETRADING_API_KEY` header
and resolve the `agent_key` internally.

## Instrument Handling

For v1, instrument handling is intentionally simple:

- Likely hardcode BTC/USD or BTC-USD-PERP until the instrument model is finalized.
- The plugin can still accept an `instrument` parameter; the model can pass it,
  but early implementations may ignore or normalize it.

## Sandbox / Paper Trading

A sandbox or paper-trading mode is a secondary feature. Possible later design:

- `HYPERLIQUID_ENVIRONMENT=testnet` as a partial sandbox
- A backend-level paper-trading flag that simulates fills without submitting to Hyperliquid

Not required for v1.

## Open Questions

1. How will the plugin be installed into Hermes from this repo? (manual copy, git tap, future API, etc.)
2. Which exact Hyperliquid Python SDK package and version should `requirements.txt` pin?
3. Which TA libraries belong in the plugin? (`pandas`, `numpy`, `ta-lib`, lightweight pure-Python alternatives?)
4. What is the exact `place_order` payload shape and response contract?
5. Should `get_account_status` cache results, or call Hyperliquid fresh every time?
6. Should the plugin provide slash commands like `/vibetrading status` or `/vibetrading balance`?
7. How should the model discover which timeframes and instruments are valid?
8. Should `fetch_candles` validate the requested interval, or let the Hyperliquid SDK error bubble up?

## Summary

The Hermes plugin for Vibetrading V2 should:

- live in `hermes-plugin/vibetrading/`
- fetch market data and account status directly from Hyperliquid
- run technical analysis locally in Python
- read and write memory through the Vibetrading backend (contracts defined, implementation deferred)
- submit orders through the Vibetrading execution gateway with links to source memory
- bundle a `trading-workflow` skill that teaches the agent the decision loop
- require `VIBETRADING_BASE_URL`, `VIBETRADING_API_KEY`, `HYPERLIQUID_ENVIRONMENT`, and `HYPERLIQUID_ADDRESS`
