# Hermes Agent Plugin

Related docs:

- `README.md` for top-level V2 direction
- `Agents.md` for the agent registry, profile mapping, and prompt split
- `Memory.md` for the append-only memory model and analysis metadata contract
- `Hyperliquid.md` for the execution gateway design

## Current Direction

The Hermes integration is now a native plugin that bundles:

- Hermes skills
- Python helper modules
- Python scripts invoked by Hermes cron pre-run hooks or by the model inside a run

This phase does not use MCP.

This phase does not register custom trading tools with Hermes.

The near-term control surface is:

- Hermes-owned cron scheduling
- Vibetrading-owned HTTP APIs for job context, memories, account state, and order execution
- a shared Hermes profile per agent, keyed by `agents.registry.agent_key`

## Plugin Location

```text
hermes-plugin/vibetrading/
```

The plugin ships in this repo so the backend and Hermes-side skill bundle stay
versioned together.

## Runtime Layout

```text
hermes-plugin/vibetrading/
├── __init__.py
├── plugin.yaml
├── requirements.txt
├── py/
│   ├── client.py
│   ├── job_context.py
│   ├── memories.py
│   ├── orders.py
│   ├── account.py
│   ├── hyperliquid_data.py
│   └── analysis.py
├── scripts/
│   ├── fetch_job_context.py
│   ├── fetch_ohlcv.py
│   ├── analyze_ohlcv.py
│   ├── read_memories.py
│   ├── write_memory.py
│   ├── list_orders.py
│   ├── place_orders.py
│   ├── cancel_orders.py
│   └── cancel_all.py
└── skills/
    ├── analysis-loop/
    │   └── SKILL.md
    └── trading-loop/
        └── SKILL.md
```

`__init__.py` registers bundled skills from `skills/*/SKILL.md`.

## Container Image

The `hermes-gateway` service in `podman-compose.yaml` builds from
`containers/hermes-agent/Dockerfile`, which layers the Vibetrading plugin onto
`nousresearch/hermes-agent:v2026.6.5`.

Important invariant:

- Hermes's own install is treated as immutable.
- Vibetrading dependencies do not get installed into `/opt/hermes/.venv`.
- The plugin owns its own venv at `/opt/data/plugins/vibetrading/.venv`.

### Plugin venv bootstrap

The image creates `/opt/data/plugins/vibetrading/.venv` at build time with
`uv`. Because development bind-mounts `./hermes-plugin/vibetrading` over the
baked directory, startup also runs an idempotent bootstrap script:

```text
/usr/local/bin/bootstrap-vibetrading-plugin
```

That script:

- creates the plugin venv if missing
- installs `requirements.txt` into that venv
- re-installs when `requirements.txt` is newer than the last install stamp

This avoids rebuilding the container for every Python dependency change during
development.

### Build and start

```bash
podman compose up --build hermes-gateway
```

The container name is `vibetrading-hermes`. The plugin source is bind-mounted
from `hermes-plugin/vibetrading/` into `/opt/data/plugins/vibetrading`.

### Verification

After startup, these checks should succeed:

```bash
podman exec vibetrading-hermes bun --version
podman exec vibetrading-hermes uv --version
podman exec vibetrading-hermes /opt/data/plugins/vibetrading/.venv/bin/python -c "import pandas, hyperliquid, talib"
podman exec vibetrading-hermes hermes plugins list | grep vibetrading
```

To confirm the dashboard is listening:

```bash
podman exec vibetrading-hermes ss -tln | grep ${HERMES_DASHBOARD_PORT:-19119}
```

## Environment Variables

The plugin currently depends on these environment variables:

| Variable | Purpose |
|----------|---------|
| `VIBETRADING_BASE_URL` | Base URL of the Vibetrading backend |
| `VIBETRADING_API_KEY` | Agent-scoped backend API key |
| `HYPERLIQUID_ENVIRONMENT` | `mainnet` or `testnet` for direct market-data reads |
| `HYPERLIQUID_ADDRESS` | Agent account address for direct exchange reads |

Provider API keys remain global to the Hermes daemon and are still passed
through from `podman-compose.yaml`.

## Hermes Profile Management

The backend provisions and manages Hermes profiles through the Hermes dashboard
API. The important behavior in this phase is:

- creating an agent attempts to create a same-named Hermes profile
- deleting an agent attempts to delete that same Hermes profile
- updating `agents.registry.soul` from the operator prompts page attempts to
  sync the Hermes profile's `SOUL.md`

Hermes failures are logged at WARN and are non-fatal to the operator action.

The `/hermes` operator page surfaces a direct dashboard link that opens in a
new tab. By default it uses the backend's configured Hermes dashboard URL; set
`APP_PUBLIC_URL` when operators browse the Vibetrading UI from another machine
so the link uses the browser-visible host instead of loopback.

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
  "account": {
    "agent_key": "btc-momentum",
    "account_address": "0x...",
    "environment": "live",
    "connected": true,
    "state": { "status": "connected" }
  }
}
```

Behavior:

- `job_kind=analysis` returns `analysis_prompt` and `account: null`
- `job_kind=trading` returns `trading_prompt` and the full current account snapshot

## Skills

Two cron-facing skills are registered by the plugin:

- `vibetrading:analysis-loop`
- `vibetrading:trading-loop`

### `analysis-loop`

- intended cadence: every 15 minutes
- intended model: stronger / more expensive
- responsibility: produce exactly one `memory_type="analysis"` memory
- must not place orders

### `trading-loop`

- intended cadence: every 1 minute
- intended model: cheaper / execution-oriented
- responsibility: consume recent analysis memory, inspect the injected account
  snapshot, and manage orders/positions through the backend only
- must never sign Hyperliquid orders directly

See `docs/Memory.md` for the reserved analysis metadata contract used as the
analysis-to-trading handoff.

## Cron Ownership

Hermes cron owns scheduling in this phase.

Vibetrading does not create, edit, pause, resume, or track Hermes cron jobs
yet.

Example analysis cron:

```bash
hermes cron create "every 15m" \
  "Run the Vibetrading analysis loop for this profile." \
  --skill vibetrading:analysis-loop \
  --script "/bin/sh -lc '/opt/data/plugins/vibetrading/.venv/bin/python /opt/data/plugins/vibetrading/scripts/fetch_job_context.py --job-kind analysis'" \
  --name "vibetrading-analysis"
```

Example trading cron:

```bash
hermes cron create "every 1m" \
  "Run the Vibetrading trading loop for this profile." \
  --skill vibetrading:trading-loop \
  --script "/bin/sh -lc '/opt/data/plugins/vibetrading/.venv/bin/python /opt/data/plugins/vibetrading/scripts/fetch_job_context.py --job-kind trading'" \
  --name "vibetrading-trading"
```

If the installed Hermes version requires scripts to live under a dedicated
Hermes scripts directory, copy or symlink the Vibetrading script there and keep
the command path explicit.

### Model split

Operationally, use:

- a stronger model for the 15 minute analysis cron
- a cheaper model for the 1 minute trading cron

If the installed Hermes version cannot override model/provider per cron job,
the current workaround is separate profiles or separate Hermes containers.

## Script Execution Contract

All bundled helper scripts should be invoked explicitly through the plugin venv:

```text
/opt/data/plugins/vibetrading/.venv/bin/python <script>
```

Scripts print JSON to stdout and errors to stderr. Non-zero exits indicate
configuration or transport failures so Hermes cron can surface them clearly.
