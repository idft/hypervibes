# VIBETRADING-V2 DESIGN

WARNING: THIS IS A WORK IN PROGRESS AND IS IDEAS ONLY.
NOTHING IN THE FILE SHOULD BE IMPLEMENTED YET.


# Overview

This app / UI should be tied into Hermes agent. It is a trading framework for Hermes.

The current V2 direction is:

- one main application binary
- Postgres as the primary database
- one normal shared migration directory for the whole app
- a dedicated `agents` schema for AI agent registry and account/instrument ownership
- a dedicated `memory` schema for the memory subsystem
- a dedicated `hyperliquid` schema for account activity and reconciliation
- a web UI plus internal APIs
- supervised internal background tasks for sync, polling, and analysis work
- NautilusTrader reused as a Hyperliquid library, not as the primary app runtime

This app should provide:

- a local Hyperliquid account history journal (HTTP-canonical ingest; websocket deferred)
- a registry of AI agents and Hermes integrations
- a structured memory system for AI agent analysis
- internal services for operator UI and agent-facing workflows

## Current Docs

- `Agents.md` - agent registry, Hermes integration, and account/instrument ownership
- `Memory.md` - memory subsystem design
- `Hyperliquid.md` - Hyperliquid account sync and reconciliation module
- `Architecture.md` - Rust-oriented implementation options and runtime shape

# Wishlist

* Record all cron job contexts -- or could get from hermes db directly??

* Single webserver / orchestrator binary? ( 1 per environment? )
  - May initially use `HYPERLIQUID_PK` from environment, but longer term account ownership should come from the agent registry.
  - Runs the app-owned execution gateway and web UI / API simultaneously.
  - only 1 "Trader" - no traders table



## Most minimal possible system

* NautilusTrader as a low-level Hyperliquid library only
* API / Webserver
* Postgres in a container
* Hermes in a container


## Memory / Analysis system

See `Memory.md`.

Current direction:

- separate memory subsystem
- dedicated `memory` schema
- scoped by `agent_key`
- higher-timeframe analysis writes structured memories
- lower-timeframe execution reads distilled active state
- daily evaluation learns from actual results

## Agents / Hermes integration

See `Agents.md`.

Current direction:

* Separate agent registry subsystem
* Dedicated `agents` schema
* Registry of AI agents, Hermes profile bindings, instrument permissions, and DB-backed config
* One execution account per agent
* One agent may trade multiple instruments, even if early testing uses only one
* Exactly one per-agent app API key for authenticated calls into the local app
* Agent API keys are app-level credentials and may be shown repeatedly in the UI without obscuring them
* Bridges `agent_key` in memory to execution-account ownership in Hyperliquid

## Web UI

* Design / Layout TODO
* Tailwind CSS
* Minimal JS first; frontend stack is still open


## Hyperliquid connection

See `Hyperliquid.md`.

Current direction:

* Separate Hyperliquid subsystem
* Dedicated `hyperliquid` schema
* Owns an internal execution gateway plus execution-intent and submitted-order records (later phase)
* Startup reconciliation before live mode
* Historical and ongoing HTTP `/info` polling for canonical account-history sync
* Websocket live fill feed deferred to a later step (polling is the v1 correctness path)
* Durable local account activity journal for fills, funding, fees, deposits, withdrawals, transfers, and historical orders
* Instrument reference sync for symbol normalization (via NautilusTrader)
* Uses NautilusTrader as a Hyperliquid venue SDK (instrument sync now; signing/order submission later), not NT strategies or `LiveNode`; account-history endpoints NT does not expose are fetched via an app-owned raw HTTP `/info` client
* Agent event webhook fanout can be added later

## Hermes Agent integration

* Hermes skills / commands / scripts as a plugin?
* Connect to hermes sqlite DB ??

## Architecture

See `Architecture.md`.

Current direction:

* Rust remains the main implementation language under consideration
* `actix-web` and `axum` are the leading Rust web options
* one binary with supervised internal tasks is currently preferred over separate deployables
