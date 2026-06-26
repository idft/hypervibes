# Hermes Profile Distribution

Related docs:

- `README.md` for top-level V2 direction
- `Agents.md` for the agent registry, profile mapping, and prompt split
- `Memory.md` for the append-only memory model and analysis metadata contract
- `Hyperliquid.md` for the execution gateway design

## Current Direction

Vibetrading no longer assumes that the backend manages a bundled Hermes runtime.

The primary integration path is now:

- users run their own Hermes instance
- users install the Vibetrading profile distribution into Hermes
- one Vibetrading backend agent maps to one Hermes profile
- the same distribution is installed once per backend agent using `--name <agent_key>`
- Hermes owns runs and cron execution
- Vibetrading owns HTTP APIs for job context, memories, account data, order execution, and loop check-ins

This phase does not use MCP.

This phase does not require Hermes plugin discovery.

## Distribution Location

```text
hermes-profile/vibetrading/
```

The distribution ships in this repo for local testing before it is split into a dedicated repository.

## Runtime Layout

```text
hermes-profile/vibetrading/
├── distribution.yaml
├── README.md
├── SOUL.md
├── config.yaml
├── requirements.txt
├── .env.EXAMPLE
├── bin/
├── cron/
├── skills/
├── scripts/
└── vibetrading/
```

The Python helpers and scripts resolve imports relative to the installed profile root, so they do not assume `~/.hermes`, `/opt/data`, or a repo checkout path.

## Install Workflow

Install the distribution into the operator's Hermes instance once per backend agent:

```bash
hermes profile install /path/to/vibetrading-v2/hermes-profile/vibetrading --name <agent_key> --alias
```

Then, in the installed profile directory:

```bash
cp .env.EXAMPLE .env
./bin/bootstrap
./bin/vibetrading-analysis
./bin/vibetrading-trading
```

Environment variables live in the installed profile's `.env` file.

## Environment Variables

The distribution currently depends on these environment variables:

| Variable | Purpose |
|----------|---------|
| `VIBETRADING_BASE_URL` | Base URL of the Vibetrading backend |
| `VIBETRADING_API_KEY` | Agent-scoped backend API key |
| `HYPERLIQUID_ENVIRONMENT` | `mainnet` or `testnet` for direct market-data reads |
| `HYPERLIQUID_ADDRESS` | Agent account address for direct exchange reads |

Provider API keys remain Hermes-side concerns and should not be hardcoded into the Vibetrading distribution.

## Hermes Profile Ownership

The backend agent row is not the source of truth for Hermes profile lifecycle.

Current ownership model:

- the operator installs the profile distribution into their own Hermes instance
- the Hermes profile name should match `agents.agent_key`
- one backend agent maps to one Hermes profile
- the backend may display setup guidance, but it should not manage Hermes profile creation, deletion, or cron state as the primary workflow

## Job Context API

Hermes cron jobs now read per-job runtime context from the backend:

```text
GET /api/v1/job-context?job_kind=analysis
GET /api/v1/job-context?job_kind=trading
```

Auth is agent-scoped bearer auth:

```http
Authorization: Bearer <agent api key>
```

Response shape:

```json
{
  "agent_key": "btc-momentum",
  "job_kind": "trading",
  "display_name": "BTC Momentum",
  "environment": "live",
  "prompt": "Manage open risk conservatively.",
  "selected_instruments": ["BTC", "ETH"],
  "instructions": [],
  "account": {
    "agent_key": "btc-momentum",
    "account_address": "0x...",
    "environment": "live",
    "account_data": {
      "available": true,
      "as_of": "2026-06-23T20:08:18.610556Z",
      "stale": false
    },
    "balance": {
      "exchange": "hyperliquid",
      "model": "unified_cross_margin",
      "total_equity_usd": "222.922072",
      "available_to_trade_usd": "222.922072",
      "available_to_withdraw_usd": "222.922072",
      "margin_used_usd": "0.0",
      "unrealized_pnl_usd": "0.0",
      "collateral_balances": [
        { "asset": "USDC", "total": "222.922072", "available": "222.922072" }
      ]
    },
    "open_positions": [],
    "open_orders": []
  }
}
```

Behavior:

- operators choose the allowed Hyperliquid perp set in the agent Settings tab
- `selected_instruments` is always returned as the currently enabled Hyperliquid perp IDs for that agent
- empty selection is valid and returns `selected_instruments: []` plus `instructions` telling the agent to do nothing until at least one currency is enabled
- `job_kind=analysis` returns `analysis_prompt` and `account: null`
- `job_kind=trading` returns `trading_prompt` plus a freshness-gated account contract containing balance, full open positions, and full open orders
- each successful `job-context` call also updates a per-loop check-in timestamp in
  `agents`

Trading agents must treat account availability as a hard safety gate:

- if `account.account_data.available != true`, do not place new opening orders
- if `account.balance == null`, do not place new opening orders
- if `selected_instruments` is empty or `instructions` says no currencies are enabled, do not analyze markets or place trades
- use `account.balance.available_to_trade_usd` for collateral sizing
- use `account.open_positions` and `account.open_orders` for exposure and order management
- raw `account.state`, raw `margin`, and raw `spot_balances` are intentionally not part of the agent-facing API contract

Order placement is also server-enforced against the same set:

- `POST /api/v1/orders` is rejected when the agent has no selected currencies
- `POST /api/v1/orders` is rejected when any submitted `symbol` is not in `selected_instruments`
- cancel endpoints are intentionally not blocked by this restriction so risk-reducing cleanup still works

New backend agents receive default strategy prompts at creation time. Those
defaults live in `src/agents/prompts.rs` and are ordinary stored prompt text,
not hard-coded strategy logic. Existing agents are not overwritten during
deploys or startup.

The operator UI uses those timestamps as a runtime health signal only:

- `analysis_context_last_used_at` tracks the analysis loop check-in
- `trading_context_last_used_at` tracks the trading loop check-in
- a setup alert means that loop has never checked in, not definite proof that a
  Hermes cron job is absent
- a stale warning means that loop has checked in before but has not checked in
  recently
- stale thresholds are 30 minutes for analysis and 3 minutes for trading

## Skills

Two cron-facing skills are shipped in the distribution:

- `analysis-loop`
- `trading-loop`

### `analysis-loop`

- intended cadence: every 15 minutes
- intended model: stronger / more expensive
- responsibility: produce exactly one `memory_type="analysis"` memory
- must not place orders

### `trading-loop`

- intended cadence: every 1 minute
- intended model: cheaper / execution-oriented
- responsibility: consume recent analysis memory, inspect the injected account
  data, balance, open positions, and open orders, and manage orders/positions through the backend only
- must never sign Hyperliquid orders directly

See `docs/Memory.md` for the reserved analysis metadata contract used as the
analysis-to-trading handoff.

The skills are workflow contracts: they enforce job-context usage, memory
handoff, account inspection, and gateway-only execution. Operator prompts from
the backend configure strategy choices, but they do not override those safety
rules.

## Cron Ownership

Hermes cron owns scheduling in this phase.

The distribution includes cron definitions under `hermes-profile/vibetrading/cron/`, and operators should review and enable them in Hermes.

Vibetrading does not create, edit, pause, resume, or inspect Hermes cron jobs directly. The backend only observes loop health through `/api/v1/job-context` check-ins.

The cron files now follow the exported Hermes shape observed in production testing:

- `name`
- `schedule`
- `prompt`
- `skills`
- `enabled_toolsets`

Current Vibetrading cron templates enable the `terminal` toolset and attach exactly one installed skill per job:

- `analysis-loop`
- `trading-loop`

### Model split

Operationally, use:

- a stronger model for the 15 minute analysis cron
- a cheaper model for the 1 minute trading cron

If the installed Hermes version cannot override model/provider per cron job,
the current workaround is separate profiles or separate Hermes containers.

## Script Execution Contract

All bundled helper scripts should be invoked explicitly through the profile-local venv from the installed Hermes profile directory:

```text
.venv/bin/python <script>
```

Scripts print JSON to stdout and errors to stderr. Non-zero exits indicate
configuration or transport failures so Hermes cron can surface them clearly.
