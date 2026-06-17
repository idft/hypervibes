# Vibetrading V2

This is a framework for allowing AI Agents (such as Hermes) to trade crypto on Hyperliquid exchange.

See `docs/` for project documentation.  Keep documentation up to date when making code changes.

The server (`cargo run`) may be running in another tmux page, for reading logs and restarting the server

## Stack

* Rust
* axum
* sqlx
* postgres
* HTMX, SSE for live updates

## Testing

Run `cargo test` from the repo root. No `podman compose`, no Docker, no external Postgres is required: DB-touching tests use an embedded `pglite-oxide` PostgreSQL server that starts in-process via `src/test_db.rs`. The dev server's container Postgres at `localhost:15432` is never touched by tests.
