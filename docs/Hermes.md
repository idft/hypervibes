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

## Container Image

The `hermes-gateway` service in `podman-compose.yaml` is built from a custom
image defined in `containers/hermes-agent/Dockerfile`. The custom image uses
`nousresearch/hermes-agent:v2026.6.5` as a base and layers on top:

- the `bun` runtime (copied from `oven/bun:1.3.14`),
- system utilities (`jq`, `git`); TA-Lib ships as a manylinux pre-compiled
  wheel on PyPI (version 0.6.8), so no system C library is needed,
- the Vibetrading plugin's Python dependencies, installed with `uv` into
  Hermes's existing `/opt/hermes/.venv`,
- a patch to `/opt/hermes/cli-config.yaml.example` so a fresh named volume
  seeds `$HERMES_HOME/config.yaml` with `vibetrading` already in
  `plugins.enabled`. The patch is YAML-aware and does not clobber existing
  upstream defaults. Once the user has run the container once, the seeded
  config.yaml persists in the `hermes_data` volume and the build-time patch
  no longer applies.

### Build and start

```bash
podman compose up --build hermes-gateway
```

This builds the custom image (or rebuilds it if the Dockerfile or plugin
source changed) and starts the `vibetrading-hermes` container in the host
network. The container name is `vibetrading-hermes` (set in
`podman-compose.yaml`) so it can coexist with another `hermes` container
already running on the host — all `podman exec` examples below use that
name. The plugin source is bind-mounted from `hermes-plugin/vibetrading/`
into `/opt/data/plugins/vibetrading` so edits on the host are
picked up on container restart without rebuilding the image. Hermes's home
directory (`/opt/data`) lives in a named volume `hermes_data` so it
survives container recreations.

### Disabling or re-enabling the plugin

The plugin is enabled on first boot by the build-time patch above. To turn
it off later (and back on) without rebuilding the image, use the Hermes CLI
inside the running container:

```bash
podman exec vibetrading-hermes hermes plugins disable vibetrading
podman exec vibetrading-hermes hermes plugins enable vibetrading
```

The change is written to `$HERMES_HOME/config.yaml` in the
`hermes_data` volume and persists across restarts.

### Container env vars

`podman-compose.yaml` exposes the following variables on the container.
All have defaults that can be overridden in `.env`:

| Variable | Default | Purpose |
|----------|---------|---------|
| `HERMES_DASHBOARD_PORT` | `19119` | Host port for the Hermes web dashboard. Upstream default is `9119`; the custom image ships shifted to `19119` to avoid colliding with another Hermes agent already bound to `9119` on the host. |
| `HERMES_DASHBOARD_SESSION_TOKEN` | *(random if omitted)* | Optional fixed session token for the dashboard's loopback/insecure auth. When set, both the container and the backend use the same value. If omitted, the dashboard generates a random token on startup and the backend scrapes it from the served `index.html`. |
| `VIBETRADING_BASE_URL` | `http://localhost:3003` | Base URL of the Vibetrading backend that the plugin's HTTP tools call. |

LLM provider API keys are also passed through from `.env` to the container
environment. Hermes reads the matching key for the provider configured on
the active profile. `podman-compose.yaml` currently forwards the following
variables; uncomment the one that matches your provider in `.env`:

- `OPENROUTER_API_KEY`
- `OPENAI_API_KEY`
- `ANTHROPIC_API_KEY`
- `GOOGLE_API_KEY`
- `GEMINI_API_KEY`
- `DEEPSEEK_API_KEY`
- `XAI_API_KEY`
- `OPENCODE_GO_API_KEY`
- `OLLAMA_API_KEY`

Provider keys are **global to the Hermes daemon** — every profile on this
container uses the same key for a given provider. If you need per-agent
provider isolation, run separate Hermes containers or use Hermes credential
pools.

Per-profile secrets — `VIBETRADING_API_KEY`, `HYPERLIQUID_ENVIRONMENT`,
`HYPERLIQUID_ADDRESS`, and any other agent-scoped values — are **not** set
on the container. They are managed through Hermes profile configuration
(see the **Hermes HTTP API (Profile Management)** section below and
`docs/Agents.md`).

### Verifying the build

After the first build and start, the following should all succeed:

```bash
podman exec vibetrading-hermes bun --version
podman exec vibetrading-hermes uv --version
podman exec vibetrading-hermes /opt/hermes/.venv/bin/python -c "import pandas, hyperliquid, talib"
podman exec vibetrading-hermes hermes plugins list | grep vibetrading
```

After the first build and start, verify that the dashboard web UI is listening on the configured port:

```bash
podman exec vibetrading-hermes ss -tln | grep ${HERMES_DASHBOARD_PORT:-19119}
```

## Environment Variables

The plugin declares these in `plugin.yaml` under `requires_env`:

| Variable | Purpose |
|----------|---------|
| `VIBETRADING_BASE_URL` | Base URL of the Vibetrading backend (e.g., `http://localhost:3000`) |
| `VIBETRADING_API_KEY` | Per-agent API key from `agents.registry.api_key`. Authenticates the agent to the backend. |
| `HYPERLIQUID_ENVIRONMENT` | `mainnet` or `testnet` |
| `HYPERLIQUID_ADDRESS` | The agent's Hyperliquid wallet / account address. Used for direct Hyperliquid reads. |

Hermes will prompt for missing variables during install and save them to `.env`.

## Hermes Web UI / Dashboard API (Profile Management)

Profile management is handled by the **Hermes web dashboard**, not by
per-profile API servers. The dashboard is a FastAPI-backed SPA served on
`HERMES_DASHBOARD_PORT` (default `19119`). It exposes the same `/api/profiles/*`
surface for both the web UI and programmatic callers.

For V2, the App's provisioning pipeline calls these dashboard endpoints to
**automatically manage each agent's Hermes profile** — creating the profile
on first registration, configuring its soul/model/description from the
agent registry, and activating it — so that spinning up a new agent in
our App end-to-end does not require any manual clicks in the Hermes
dashboard.

The public liveness endpoint is `GET /api/status`.

### Profile endpoints

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `GET` | `/api/profiles` | List all installed profiles |
| `POST` | `/api/profiles` | Create a new profile |
| `GET` | `/api/profiles/active` | Get the currently active profile |
| `POST` | `/api/profiles/active` | Set the active profile (`{"name": "..."}`) |
| `GET` | `/api/profiles/{name}` | Implicit via the routes below |
| `PATCH` | `/api/profiles/{name}` | Rename a profile (`{"new_name": "..."}`) |
| `DELETE` | `/api/profiles/{name}` | Delete a profile |
| `GET` | `/api/profiles/{name}/soul` | Read the profile's soul / persona text |
| `PUT` | `/api/profiles/{name}/soul` | Replace the profile's soul (`{"content": "..."}`) |
| `PUT` | `/api/profiles/{name}/description` | Set the profile description (`{"description": "..."}`) |
| `PUT` | `/api/profiles/{name}/model` | Set provider + model (`{"provider": "...", "model": "..."}`) |
| `POST` | `/api/profiles/{name}/describe-auto` | Auto-generate a description via the auxiliary LLM (`{"overwrite": false}`) |
| `GET` | `/api/profiles/{name}/setup-command` | Get the CLI setup command for the profile |
| `POST` | `/api/profiles/{name}/open-terminal` | Open a terminal session for the profile |
| `GET` | `/api/profiles/sessions` | List sessions, filterable by profile |

A smaller set of profile routes also exists under the Kanban plugin for the
board's assignee picker (e.g. `GET /api/plugins/kanban/profiles`); the
plugin should treat those as read-only.

### Request body schemas

- **`ProfileCreate`** (`POST /api/profiles`) — only `name` is required:
  - `name`: string *(required)*
  - `clone_from_default`: bool (default `false`)
  - `clone_all`: bool (default `false`)
  - `no_skills`: bool (default `false`)
  - `description`: string | null
  - `clone_from`: string | null *(clone from another named profile)*
  - `provider`: string | null
  - `model`: string | null
- **`ProfileActiveUpdate`** — `{ "name": "..." }`
- **`ProfileRename`** — `{ "new_name": "..." }`
- **`ProfileSoulUpdate`** — `{ "content": "..." }`
- **`ProfileDescriptionUpdate`** — `{ "description": "..." }` (default `""` to clear)
- **`ProfileModelUpdate`** — `{ "provider": "...", "model": "..." }`
- **`ProfileDescribeAuto`** — `{ "overwrite": false }`

### Automatic profile management flow

When a new agent is registered in our App, the **App's provisioning
pipeline** should call the Hermes API in this order so the Hermes profile
exists, is configured, and is the active one before the agent ever runs a
turn:

1. `GET /api/profiles` — check whether a profile for this agent already
   exists (matched by name against `agents.hermes_bindings.hermes_profile`).
2. `POST /api/profiles` with `ProfileCreate` if missing — typically
   `clone_from_default=true` so it inherits a working baseline.
3. `PUT /api/profiles/{name}/soul` with the agent's persona text sourced
   from the registry (so each agent has a distinct, auditable soul).
4. `PUT /api/profiles/{name}/model` with the provider/model assigned to
   the agent.
5. `PUT /api/profiles/{name}/description` with a short human-readable
   description (or `POST /api/profiles/{name}/describe-auto` to let the
   auxiliary LLM draft one).
6. `POST /api/profiles/active` with `{"name": "..."}` so subsequent
   Hermes runs use this profile.

Idempotency: every step above should be safe to re-run on a profile that
already exists / is already configured. The plugin should treat a 200
response with the same shape as a no-op success.

Auth: in loopback / `--insecure` mode, the dashboard injects an ephemeral
`__HERMES_SESSION_TOKEN__` into the served SPA. Programmatic callers must
send it back in the `X-Hermes-Session-Token` header (legacy `Authorization: Bearer`
is also accepted). The easiest way for the backend to stay authenticated is
to set the same `HERMES_DASHBOARD_SESSION_TOKEN` on both the container and
the backend; if that variable is omitted, the backend scrapes the token
from `GET /` once on first use. In gated / OAuth mode the token is not
injected; callers must authenticate through the dashboard's OAuth gate, which
is outside the scope of the backend integration.

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

Endpoint: `POST /api/v1/orders` (see the wire contract below).

Inputs (per `PlaceOrderInput`):

- `symbol` (e.g. `"BTC"`)
- `side` — `"buy"` or `"sell"`
- `order_type` — `"limit"` or `"market"`
- `size` (decimal string, e.g. `"0.1"`)
- `price` (decimal string) — required for `limit`, ignored for `market`
- `time_in_force` — `"gtc"` (default for limit), `"ioc"`, `"alo"`
- `reduce_only` (boolean, default `false`)
- `take_profits` — list of `{trigger_price, limit_price?, size?}`; TP legs are
  always reduce-only and on the opposite side of the entry.
- `stop_losses` — list of `{trigger_price, limit_price?, size?}`; SL legs are
  always stop-market and reduce-only on the opposite side.
- `memory_record_ids` — list of memory record IDs to link the order to.

Output (`PlaceOrdersResponse`):

```json
{
  "results": [
    {
      "id": "uuid",
      "cloid": "0x...",
      "symbol": "BTC",
      "side": "buy",
      "order_kind": "limit",
      "status": "resting",
      "exchange_oid": "12345",
      "group_id": "uuid|null",
      "error": null
    }
  ]
}
```

Notes:

- The backend gateway records every leg as its own row in
  `hyperliquid.orders` before forwarding to Hyperliquid.
- The HTTP response status is `201 Created` on success.
- A failed individual leg has `status: "error"` or `"rejected"` and a
  non-null `error`; other legs may still succeed.
- Linking the order to source memory is a core idea; pass
  `memory_record_ids` when available.

### `cancel_order`

Cancels one or more orders by exchange `oid`.

Endpoint: `POST /api/v1/orders/cancel`

Inputs:

```json
{
  "orders": [
    { "symbol": "BTC", "oid": 12345 }
  ]
}
```

Output: a JSON array of per-input outcomes, each with `symbol`, `oid`,
`status` (`"submitted"`/`"canceled"`/`"rejected"`/`"not_found"`/`"error"`),
optional `error` string, and `order_id` of the local row when known.

### `cancel_all`

Cancels every resting order for the calling agent. Optionally restrict
to one symbol.

Endpoint: `POST /api/v1/orders/cancel-all?symbol=BTC`

Output:

```json
{
  "considered": 3,
  "outcomes": [ ... ]
}
```

### `list_orders`

Lists the agent's orders, newest first. Scoped by `agent_key` server-side.

Endpoint: `GET /api/v1/orders?status=open&symbol=BTC`

- `status` — `open` (alias for `submitted|resting|partially_filled`),
  `pending`, or any exact status name. Omit for all.
- `symbol` — optional symbol filter.

Output: JSON array of order rows.

Use this to find the `exchange_oid` you need for `cancel_order` (it
appears as the `exchange_oid` field on each row).

### `get_order`

Fetches a single order by id. Optional `?include=events` appends the
lifecycle events from `hyperliquid.order_events`.

Endpoint: `GET /api/v1/orders/{id}?include=events`

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
| `GET /api/v1/account` | `get_account_state` |
| `POST /api/v1/orders` | `place_order` |
| `GET /api/v1/orders` | `list_orders` |
| `GET /api/v1/orders/{id}?include=events` | `get_order` (lifecycle) |
| `POST /api/v1/orders/cancel` | `cancel_order` |
| `POST /api/v1/orders/cancel-all` | `cancel_all` |

All agent-facing endpoints authenticate via the `Authorization: Bearer
{VIBETRADING_API_KEY}` header and resolve the `agent_key` internally.

## Implementing a trading skill (Python)

A Hermes skill that trades should call the Vibetrading execution
gateway rather than Hyperliquid directly. This section shows the
canonical pattern using `requests` (or `httpx`, with the same shape).

### Required environment

| Var | Purpose |
|-----|---------|
| `VIBETRADING_BASE_URL` | e.g. `https://vibetrading.example.com` |
| `VIBETRADING_API_KEY` | the agent's API key (issued by the operator) |

### Minimal client module

```python
# vibetrading_client.py
import os, json
from typing import Any
import requests

BASE = os.environ["VIBETRADING_BASE_URL"].rstrip("/")
KEY = os.environ["VIBETRADING_API_KEY"]

def _headers() -> dict[str, str]:
    return {
        "Authorization": f"Bearer {KEY}",
        "Content-Type": "application/json",
    }

def place_orders(orders: list[dict]) -> list[dict]:
    """POST /api/v1/orders. Returns the per-leg results list."""
    r = requests.post(f"{BASE}/api/v1/orders",
                      headers=_headers(),
                      data=json.dumps({"orders": orders}),
                      timeout=10)
    if r.status_code == 401:
        raise PermissionError("invalid VIBETRADING_API_KEY")
    if r.status_code == 422:
        raise ValueError(r.json().get("error", "validation failed"))
    r.raise_for_status()
    return r.json()["results"]

def list_orders(status: str | None = None,
                symbol: str | None = None) -> list[dict]:
    """GET /api/v1/orders."""
    params: dict[str, Any] = {}
    if status: params["status"] = status
    if symbol: params["symbol"] = symbol
    r = requests.get(f"{BASE}/api/v1/orders",
                     headers=_headers(), params=params, timeout=10)
    r.raise_for_status()
    return r.json()

def cancel_orders(orders: list[dict]) -> list[dict]:
    """POST /api/v1/orders/cancel."""
    r = requests.post(f"{BASE}/api/v1/orders/cancel",
                      headers=_headers(),
                      data=json.dumps({"orders": orders}),
                      timeout=10)
    r.raise_for_status()
    return r.json()

def cancel_all(symbol: str | None = None) -> dict:
    """POST /api/v1/orders/cancel-all."""
    params: dict[str, Any] = {}
    if symbol: params["symbol"] = symbol
    r = requests.post(f"{BASE}/api/v1/orders/cancel-all",
                      headers=_headers(), params=params, timeout=10)
    r.raise_for_status()
    return r.json()
```

### Example payloads

Plain limit buy:

```python
results = place_orders([{
    "symbol": "BTC",
    "side": "buy",
    "order_type": "limit",
    "size": "0.1",
    "price": "50000",
    "time_in_force": "gtc",
}])
```

Market order:

```python
results = place_orders([{
    "symbol": "ETH",
    "side": "buy",
    "order_type": "market",
    "size": "0.5",
}])
```

Limit entry with one take-profit and one stop-loss:

```python
results = place_orders([{
    "symbol": "BTC",
    "side": "buy",
    "order_type": "limit",
    "size": "0.1",
    "price": "50000",
    "time_in_force": "gtc",
    "take_profits": [{
        "trigger_price": "55000",
        "limit_price": "55100",   # if omitted, treated as market-on-trigger
    }],
    "stop_losses": [{
        "trigger_price": "48000",
        # size omitted -> inherits the entry size
    }],
}])
# `results` has three entries: the entry, the TP, the SL. All three
# share the same `group_id`.
```

Batch of two independent orders:

```python
results = place_orders([
    {"symbol": "BTC", "side": "buy",  "order_type": "limit", "size": "0.1", "price": "50000"},
    {"symbol": "ETH", "side": "sell", "order_type": "limit", "size": "1.0", "price": "3500"},
])
```

Cancel by `oid` (look up the `oid` from `list_orders` first):

```python
open_orders = list_orders(status="open", symbol="BTC")
to_cancel = [{"symbol": o["symbol"], "oid": int(o["exchange_oid"])}
             for o in open_orders if o["exchange_oid"]]
outcomes = cancel_orders(to_cancel)
```

### Error handling

- `201 Created` on a successful `place_orders` call. Per-leg errors
  appear in each result's `error` field — the request itself does not
  fail unless the request body is invalid.
- `422 Unprocessable Entity` on a bad body (empty orders, bad side,
  limit without price, etc.) — surface the `error` string to the model
  so it can fix and retry.
- `401 Unauthorized` on a bad or missing API key.
- `5xx` for transient server errors — retry with backoff.

### Where this fits in the `trading-workflow` skill

After `analyze_market` and `read_memory` and once the model decides
to trade:

1. Optionally call `write_memory` first to record the thesis.
2. Call `place_orders` with the new memory record id in
   `memory_record_ids` so the order is linked to its justification.
3. On a non-`resting`/`filled` status, optionally call `cancel_orders`
   by `exchange_oid` to flatten the position.
4. Call `write_memory` again with the post-trade outcome (entry price,
   status, group id).

Always prefer `cancel_orders` to manual retries when a leg is
rejected; the gateway records the per-leg error in `order_events` and
is idempotent.

## Hermes Profile Management Contracts

The plugin (or the App's provisioning pipeline) will call these Hermes
endpoints to keep the agent's Hermes profile in sync with the registry.
See the **Hermes HTTP API (Profile Management)** section above for the
full surface; the minimum set the V2 App will exercise is:

| Endpoint | When |
|----------|------|
| `GET /api/profiles` | On agent boot, to check existence |
| `POST /api/profiles` | On first registration, to create the profile |
| `PUT /api/profiles/{name}/soul` | On registration, to seed persona |
| `PUT /api/profiles/{name}/model` | On registration and on any model reassignment |
| `PUT /api/profiles/{name}/description` | On registration, to attach a human description |
| `POST /api/profiles/active` | On agent boot, to ensure the right profile is active |
| `DELETE /api/profiles/{name}` | On agent de-registration |

### Backend-managed profile lifecycle (implemented)

The Vibetrading backend now automatically manages the Hermes profile for
every agent in `agents.registry`. Operator creation of an agent via the
web UI is sufficient — there is no separate Hermes dashboard step.

**Mapping**: `agent_key == hermes profile name`. The agent registry row
is the single source of truth for an agent's identity, and the Hermes
profile is its runtime counterpart.

**On agent create** (the operator form `POST /agents`):

1. The new row is inserted into `agents.registry` as before.
2. If a Hermes client is configured (see `HERMES_DASHBOARD_*` env vars
   below), the backend calls `POST /api/profiles` on the Hermes dashboard
   with `{"name": <agent_key>, "clone_from_default": true}`.
3. On success, the backend calls `PUT /api/profiles/{name}/soul` with
   the contents of the registry row's `soul` column. This is the
   operator-entered persona text from the new agent form.
4. The backend intentionally does **not** call `POST /api/profiles/active`.
   Creating an agent must not silently change the currently active
   Hermes profile; activation is an operator decision, made from the
   Hermes dashboard or via a future explicit endpoint.

**On agent delete** (`POST /agents/{agent_key}/delete`):

1. The row is deleted from `agents.registry` first.
2. If a Hermes client is configured, the backend calls
   `DELETE /api/profiles/{agent_key}` as a best-effort cleanup.
3. Profile-delete failures are logged at WARN and do not roll back the
   registry delete; the agent is gone from Vibetrading either way.

**Resilience**: every Hermes call is wrapped in a `tracing::warn!` on
failure. The agent create/delete still succeeds when Hermes is
unreachable, unconfigured, or returns an error. A future operator
reconciliation job can re-attempt profile create/delete for any drift.

**Configuration**: the Hermes client targets the dashboard web UI and is
optional. When `HERMES_DASHBOARD_HOST` and `HERMES_DASHBOARD_PORT` are
reachable they are used. `HERMES_DASHBOARD_SESSION_TOKEN` is sent as the
`X-Hermes-Session-Token` header; when omitted, the backend reads the token
from the dashboard's served `index.html`. If the Hermes client fails to
construct (e.g. malformed session token), the server logs a warning at
startup and disables Hermes integration for that run; the rest of the
server works normally. The `/hermes` operator page renders the
"not configured" state in that case.

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
4. What is the exact `place_order` payload shape and response contract? *(Answered: see the `place_order` section above and the Python skill section.)*
5. Should `get_account_status` cache results, or call Hyperliquid fresh every time?
6. Should the plugin provide slash commands like `/vibetrading status` or `/vibetrading balance`?
7. How should the model discover which timeframes and instruments are valid?
8. Should `fetch_candles` validate the requested interval, or let the Hyperliquid SDK error bubble up?
9. What auth scheme does a gated Hermes daemon require for `/api/profiles/*`?
   (`Authorization: Bearer`? session cookie? short-lived WS-style ticket?)
10. Should the App's provisioning pipeline call the Hermes API directly, or
    should the plugin expose a one-shot `ensure_profile` tool that the App
    invokes over its plugin IPC?
11. Where does the agent's soul text live canonically — the Vibetrading
    registry, or a file the plugin reads at boot? Today the doc assumes
    the registry, but the Hermes dashboard edits it on disk.

## Summary

The Hermes plugin for Vibetrading V2 should:

- live in `hermes-plugin/vibetrading/`
- fetch market data and account status directly from Hyperliquid
- run technical analysis locally in Python
- read and write memory through the Vibetrading backend (contracts defined, implementation deferred)
- submit orders through the Vibetrading execution gateway with links to source memory
- bundle a `trading-workflow` skill that teaches the agent the decision loop
- require `VIBETRADING_BASE_URL`, `VIBETRADING_API_KEY`, `HYPERLIQUID_ENVIRONMENT`,
  `HYPERLIQUID_ADDRESS`, `HERMES_BASE_URL`, and (optionally) `HERMES_API_KEY`
