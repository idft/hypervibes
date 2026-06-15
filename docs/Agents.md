# Agents Module

Related docs:

- `README.md` for top-level V2 direction
- `Memory.md` for analysis and memory ownership per agent
- `Hyperliquid.md` for execution-account, order, and reconciliation ownership
- `Architecture.md` for implementation-language and runtime options

## Goal

Define the agent registry subsystem for V2.

This module should be the system-of-record for:

- which AI agents exist
- how they map to Hermes profiles and integrations
- which instrument or market each agent is allowed to trade
- which Hyperliquid execution account each agent uses
- which API identity each agent uses to call the internal app
- which shared skills and profile-level configuration each agent runs with

This module should sit between the memory subsystem and the Hyperliquid subsystem.

## Phase 1 Implementation Note

The current Phase 1 implementation follows
`.opencode/plans/agents-implementation-plan.md` and intentionally uses a
simplified single-table registry (`agents.registry`) with:

- no instrument bindings or per-agent instrument restrictions
- no environment field; everything is implicitly `live`
- no Hermes profile column or Hermes bindings table
- no secret refs table
- a boolean `enabled` flag in place of a lifecycle enum
- one app API key stored directly on the registry row
- the Hyperliquid private key encrypted at the application layer before storage

The broader schema ideas below describe future directions rather than the
running Phase 1 data model.

## Why This Module Exists

The current design already has two different ownership boundaries:

- memory is scoped by `agent_key`
- Hyperliquid history is scoped by `account_address` + `environment`

Those two concepts need an explicit bridge.

Without an agents module, the system cannot clearly answer:

- which agent owns which memory stream
- which account that agent trades with
- which Hermes profile that agent corresponds to
- which instrument set that agent is allowed to trade
- which Hermes integration is associated with that agent

## Current Direction

Current design assumptions:

- one execution account per agent
- each agent represents a `Profile` in a Hermes agent instance
- all agents have the same installed skill surface for interacting with this app
- one agent may trade multiple instruments
- initial testing will likely still use one primary instrument such as `BTC-USD-PERP.HYPERLIQUID`
- agent registry should support multiple agents from the start
- the agent should know its account address, but should not directly hold the Hyperliquid private key

This means the effective chain becomes:

- `Hermes profile -> agent registry entry -> memory stream -> execution account -> Hyperliquid activity history`

## Primary Abstraction

The primary abstraction should be:

- one agent per Hyperliquid execution account

Not:

- one agent per instrument

An agent may still have a narrow allowed instrument set.

For early testing, that set may contain only one market such as `BTC-USD-PERP.HYPERLIQUID`.

But the registry should treat the account as the stronger ownership boundary.

This is important because:

- Hyperliquid reconciliation happens at the account level
- execution intent and order ownership are account-scoped
- evaluation ultimately ties back to account-level history
- expanding from one instrument to multiple instruments should not require redefining the agent model

## Scope

The agents module should own:

- agent identity and metadata
- Hermes profile bindings
- agent to instrument mappings
- agent to execution-account mappings
- internal API key identity for each agent
- optional secret references or encrypted credentials if the system later stores them in Postgres

The agents module should not own:

- raw market memory records
- Hyperliquid exchange history
- fills, funding, or ledger reconciliation

## Main Responsibilities

### 1. Agent Registry

Track each AI agent as a first-class entity.

Each registry entry should represent one Hermes profile.

An agent should have:

- stable `agent_key`
- lifecycle status
- environment such as `live`, `sandbox`, or `paper`
- Hermes profile identity
- allowed instrument set
- execution account reference
- internal API identity
- Hermes integration metadata

### 2. Hermes Profile Mapping

Each agent should correspond to one Hermes profile.

That profile is the unit that runs prompts and skills inside Hermes.

Current direction:

- one registry agent row == one Hermes profile
- the app does not need a separate abstraction below the Hermes profile level right now
- if Hermes later supports multiple useful layers, this can be expanded

This also means:

- the Hermes profile is the natural unit that receives the agent API key
- the Hermes profile is the natural unit that calls memory and execution APIs
- the Hermes profile is the natural owner of one memory stream via `agent_key`

### 3. Hermes Integration Mapping

This module should define how an agent is connected to Hermes.

Examples:

- Hermes profile name
- shared skill bundle
- command set
- plugin or script references
- prompt version references

Current direction:

- all agents will have the same installed skill surface
- those skills describe how to interact with the local app APIs
- differences between agents should mostly come from profile configuration, memory, account, and allowed market scope rather than from completely different installed skills
- agent configuration can live in the database rather than in static config files

Example skill categories:

- retrieving memories
- storing memories
- placing orders through the execution gateway
- fetching historical data
- analyzing historical data

The exact Hermes integration format is still open, but the installed skill surface should be mostly uniform.

This should keep the system simpler:

- one common API contract for all agents
- one common skill surface
- different behavior comes from data and profile context, not from bespoke per-agent tooling

### 4. Account Mapping

Each agent should map to one Hyperliquid execution account.

Current direction:

- one execution account per agent

This keeps execution ownership clear and makes later evaluation easier.

Important clarification:

- the app may hold the private key or secret reference needed for execution
- the agent itself should not directly hold the private key
- the agent should only operate through its app identity and account mapping

The agent can know and reference its account address.

### 5. Instrument Responsibility

The module should record which instruments or markets the agent is allowed to trade.

The earlier idea was one agent per instrument and account.

Current direction has shifted:

- one agent per Hyperliquid account
- that agent may trade multiple instruments
- for early testing, the allowed set may still contain only one instrument such as `BTC-USD-PERP.HYPERLIQUID`

This is a better abstraction because the execution account is the stronger ownership boundary than a single instrument.

The schema should therefore allow:

- one agent trading one instrument
- one agent trading multiple instruments
- multiple agents on the same instrument in different environments

Current recommended runtime policy:

- support multiple instruments per agent in the data model
- keep the allowed set to one primary instrument during early testing
- widen the allowed set later without redesigning the registry

## API Identity Model

Each agent should have an API key for calling the local application.

All agent-facing API endpoints should require an API key.

The system should resolve the agent from that API key and derive:

- `agent_key`
- execution account mapping
- allowed instruments
- profile-level permissions or environment restrictions

This API key is not the same thing as a Hyperliquid private key.

It is an app-level identity token.

### Display Policy

Current direction:

- this API key does not need a strict "show only once" UX
- it may be displayed repeatedly in the web UI
- it may be copied again later by an operator

Because of that, the storage model must allow retrieval.

That means the app should not rely on a write-only hashed token model.

Current direction:

- store the API key directly in the database
- allow the UI to display it whenever needed
- do not use a show-once-only UX

### Important Practical Note

Even if this API key is treated as low-sensitivity and is easy to view in the UI, it still authorizes actions on behalf of that agent.

So it should still be:

- revocable
- replaceable
- scoped to one agent
- auditable when used

The system can choose to display it more freely than a true exchange secret, but it should still be treated as a bearer credential.

### Recommended Behavior

Recommended API key behavior:

- exactly one API key per agent
- each key resolves to exactly one `agent_key`
- keys should be revocable
- replacing a key should be a simple regenerate operation
- the system should record last-used time and possibly recent usage metadata
- all agent-facing endpoints should authenticate through this key before resolving account or instrument permissions

## Runtime Resolution Flow

The agents module should define the runtime identity flow for incoming agent requests.

Recommended flow:

1. Hermes profile calls the local app with its agent API key.
2. The app resolves `agent_key` from that API key.
3. The app loads the agent registry entry.
4. The app resolves the execution account for that agent.
5. The app resolves the allowed instrument set for that agent.
6. The app applies any environment or profile restrictions.
7. The request is forwarded to the relevant subsystem, such as memory or the Hyperliquid execution gateway.

This means the rest of the app should not need to guess who the caller is.

The caller identity should already be resolved at the boundary.

## Agent Lifecycle

Each agent should have a simple lifecycle.

Suggested conceptual states:

- `active`
- `disabled`
- `archived`

Meaning:

- `active`: may authenticate, read memory, and submit actions within its permissions
- `disabled`: registry entry is retained, but API access and execution should be blocked
- `archived`: historical reference only

This lifecycle should be independent from whether the execution account currently has open positions or recent activity.

## Secrets and Credentials

The agent registry may eventually need to reference Hyperliquid API wallet secrets or related credentials.

If secrets are ever stored in Postgres, they should not be stored in plaintext.

Preferred direction:

- store ciphertext in Postgres
- use application-level envelope encryption
- keep the master encryption key outside the database
- allow rows to store secret references, not only raw secret values

This is possible, but should be treated carefully.

For the first version, it may still be simpler to load the active secret from environment or deployment config and let the database store only references or metadata.

This means there are two distinct credential classes:

1. Hyperliquid execution secret used by the app or execution gateway
2. local app API key used by the agent to authenticate to the app

Those should not be conflated.

Recommended ownership:

- the app or execution gateway owns exchange-signing authority
- the agent only owns app-level identity

## Proposed Schema Direction

Recommended schema name:

- `agents`

### Core Tables

- `agents.registry`
- `agents.hermes_bindings`
- `agents.instrument_bindings`
- `agents.execution_accounts`
- `agents.api_keys`
- `agents.secret_refs`

Optional later tables:

- `agents.skill_assignments`
- `agents.webhook_registrations`
- `agents.prompt_profiles`

## Core Table Ideas

### `agents.registry`

Purpose:

- canonical registry of known AI agents

Suggested fields:

- `agent_key TEXT PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `updated_at TIMESTAMPTZ NOT NULL`
- `status TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `display_name TEXT`
- `agent_role TEXT`
- `hermes_profile TEXT NOT NULL`
- `account_address TEXT`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

### `agents.hermes_bindings`

Purpose:

- map one agent to Hermes-specific runtime configuration

Suggested fields:

- `agent_key TEXT NOT NULL`
- `hermes_profile TEXT`
- `shared_skill_bundle TEXT`
- `plugin_ref TEXT`
- `command_ref TEXT`
- `prompt_profile TEXT`
- `config JSONB NOT NULL DEFAULT '{}'::jsonb`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

### `agents.instrument_bindings`

Purpose:

- define which instrument or market an agent is allowed to trade

Suggested fields:

- `agent_key TEXT NOT NULL`
- `instrument_key TEXT NOT NULL`
- `symbol TEXT NOT NULL`
- `market_type TEXT`
- `is_primary BOOLEAN NOT NULL DEFAULT true`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

This should line up later with the instrument reference table in the Hyperliquid subsystem.

### `agents.execution_accounts`

Purpose:

- map an agent to the Hyperliquid execution account it controls

Suggested fields:

- `agent_key TEXT NOT NULL`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `is_primary BOOLEAN NOT NULL DEFAULT true`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Current direction is one primary execution account per agent. The natural account identity is `account_address` + `environment`; a dedicated UUID `account_id` is intentionally not introduced in phase 1.

### `agents.api_keys`

Purpose:

- app-level agent authentication
- resolve incoming agent API requests to `agent_key`

Suggested fields:

- `id UUID PRIMARY KEY`
- `agent_key TEXT NOT NULL`
- `created_at TIMESTAMPTZ NOT NULL`
- `status TEXT NOT NULL`
- `api_key TEXT NOT NULL`
- `label TEXT`
- `last_used_at TIMESTAMPTZ`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Recommended direction:

- store the API key directly
- resolve incoming requests directly from that stored key
- allow the UI to display the exact key whenever needed

### `agents.secret_refs`

Purpose:

- optional registry for encrypted or externally managed secret references

Suggested fields:

- `id UUID PRIMARY KEY`
- `agent_key TEXT NOT NULL`
- `secret_kind TEXT NOT NULL`
- `storage_kind TEXT NOT NULL`
- `ciphertext BYTEA`
- `key_id TEXT`
- `external_ref TEXT`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

The exact implementation can be decided later.

## Draft Relational Schema

This section turns the table ideas above into a more concrete first-pass schema draft.

The goal is to define a clean registry model for:

- agent identity
- Hermes profile ownership
- account ownership
- instrument permissions
- app-level authentication
- DB-backed agent configuration

### General Conventions

Use these general rules:

- `TEXT` primary key for `agent_key`
- `UUID` primary keys for secondary tables
- `TIMESTAMPTZ` for all timestamps
- `TEXT` plus `CHECK` constraints for status-like fields in the first version
- `JSONB` for flexible metadata

Suggested common columns:

- `created_at TIMESTAMPTZ NOT NULL`
- `updated_at TIMESTAMPTZ NOT NULL` where rows can change over time
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

### 1. `agents.registry`

Suggested columns:

- `agent_key TEXT PRIMARY KEY`
- `created_at TIMESTAMPTZ NOT NULL`
- `updated_at TIMESTAMPTZ NOT NULL`
- `status TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `display_name TEXT`
- `agent_role TEXT`
- `hermes_profile TEXT NOT NULL`
- `account_address TEXT`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:

- `UNIQUE (environment, hermes_profile)`
- `CHECK (status IN ('active', 'disabled', 'archived'))`

### 2. `agents.hermes_bindings`

Suggested columns:

- `agent_key TEXT PRIMARY KEY`
- `hermes_profile TEXT NOT NULL`
- `shared_skill_bundle TEXT`
- `plugin_ref TEXT`
- `command_ref TEXT`
- `prompt_profile TEXT`
- `config JSONB NOT NULL DEFAULT '{}'::jsonb`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:

- `FOREIGN KEY (agent_key) REFERENCES agents.registry(agent_key)`

### 3. `agents.instrument_bindings`

Suggested columns:

- `id UUID PRIMARY KEY`
- `agent_key TEXT NOT NULL`
- `instrument_key TEXT NOT NULL`
- `symbol TEXT NOT NULL`
- `market_type TEXT`
- `is_primary BOOLEAN NOT NULL DEFAULT false`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:

- `FOREIGN KEY (agent_key) REFERENCES agents.registry(agent_key)`
- `UNIQUE (agent_key, instrument_key)`

Notes:

- this table should allow multiple instruments per agent
- early testing can still use only one row per agent
- `instrument_key` should line up later with `hyperliquid.instruments` ownership

### 4. `agents.execution_accounts`

Suggested columns:

- `id UUID PRIMARY KEY`
- `agent_key TEXT NOT NULL`
- `account_address TEXT NOT NULL`
- `environment TEXT NOT NULL`
- `is_primary BOOLEAN NOT NULL DEFAULT true`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:

- `FOREIGN KEY (agent_key) REFERENCES agents.registry(agent_key)`
- `UNIQUE (agent_key, account_address, environment)`

Current direction is one primary execution account per agent. Phase 1 keeps the natural account identity (`account_address` + `environment`) and does not reference a `hyperliquid.accounts` UUID table.

### 5. `agents.api_keys`

Suggested columns:

- `id UUID PRIMARY KEY`
- `agent_key TEXT NOT NULL`
- `created_at TIMESTAMPTZ NOT NULL`
- `status TEXT NOT NULL`
- `api_key TEXT NOT NULL`
- `label TEXT`
- `last_used_at TIMESTAMPTZ`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:

- `FOREIGN KEY (agent_key) REFERENCES agents.registry(agent_key)`
- `UNIQUE (agent_key)`
- `UNIQUE (api_key)`
- `CHECK (status IN ('active', 'revoked', 'archived'))`

Recommended behavior:

- look up or verify incoming credentials using `api_key`
- re-display the exact key in the UI directly
- enforce exactly one API key per agent

### 6. `agents.secret_refs`

Suggested columns:

- `id UUID PRIMARY KEY`
- `agent_key TEXT NOT NULL`
- `secret_kind TEXT NOT NULL`
- `storage_kind TEXT NOT NULL`
- `ciphertext BYTEA`
- `key_id TEXT`
- `external_ref TEXT`
- `metadata JSONB NOT NULL DEFAULT '{}'::jsonb`

Suggested constraints:

- `FOREIGN KEY (agent_key) REFERENCES agents.registry(agent_key)`

This table should represent secrets used by the app or execution gateway, not by the agent itself.

### Suggested Initial Indexes

For `agents.registry`:

- `(environment, status)`
- `(environment, hermes_profile)` unique

For `agents.hermes_bindings`:

- primary key `(agent_key)`

For `agents.instrument_bindings`:

- `(agent_key, instrument_key)` unique
- `(agent_key, is_primary)`
- `(symbol)`

For `agents.execution_accounts`:

- `(agent_key)`
- `(account_address, environment)`

For `agents.api_keys`:

- `(api_key)` unique
- `(agent_key)` unique
- `(status)`
- `(last_used_at DESC)`

For `agents.secret_refs`:

- `(agent_key, secret_kind)`

### Example Runtime Resolution

1. Hermes profile calls the app with an agent API key.
2. The app resolves the key against `agents.api_keys`.
3. The app loads `agents.registry` for the resolved `agent_key`.
4. The app loads `agents.execution_accounts` to resolve the execution account.
5. The app loads `agents.instrument_bindings` to enforce allowed instruments.
6. The app routes the request to memory APIs or the Hyperliquid execution gateway.

## Cross-Subsystem Role

The agents module should provide the missing join model between the other two major subsystems.

Examples:

- `Memory.agent_key -> Agents.registry.agent_key`
- `Agents.execution_accounts.account_address + environment -> Hyperliquid journal history`
- `Agents.api_keys -> Agents.registry.agent_key`

That gives the system a clean path from:

- memory hypothesis
- execution decision
- order submission
- exchange result
- evaluation

## Query Requirements

This module should make these lookups easy:

- resolve agent by `agent_key`
- resolve agent by API key
- resolve agent by environment and symbol
- resolve execution account for an agent
- resolve Hermes profile or skill bundle for an agent
- resolve instrument responsibility for an agent
- resolve whether a requested instrument is inside the agent's allowed set

## Open Questions

1. Should the app keep a `label` or human-friendly purpose field for the single API key?
2. Should the agent registry store only account references, or also secret references used by the app or execution gateway?
3. Should webhook registrations live here from the start, or remain deferred?

## Summary

The agents module should be a dedicated `agents` schema that defines who the AI agents are and how they connect to Hermes profiles, instruments, API identity, and Hyperliquid execution accounts.

This module fills the current gap between:

- memory ownership by `agent_key`
- Hyperliquid ownership by `account_address` + `environment`

It should be the canonical registry layer for agent identity and execution-account mapping.

Current direction is:

- one agent == one Hermes profile
- one agent == one Hyperliquid execution account
- one agent may trade multiple instruments
- agents authenticate to the app using one visible per-agent API key
- agent configuration can live in the database
