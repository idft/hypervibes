# Vibetrading V2

This is a framework for allowing AI Agents (such as Hermes) to trade crypto on Hyperliquid exchange.
The framework uses the `hypersdk` Rust crate to place and track orders on Hyperliquid.

See `docs/` for project documentation.

The server (`cargo run`) may be running in another tmux page, for reading logs and restarting the server

## Testing

Run `cargo test` from the repo root. No `podman compose`, no Docker, no external Postgres is required: DB-touching tests use an embedded `pglite-oxide` PostgreSQL server that starts in-process via `src/test_db.rs`. The dev server's container Postgres at `localhost:15432` is never touched by tests.

### Known-ignored tests

Seven SSE body-read tests in `src/web/routes.rs` are marked `#[ignore]` because the body reader (a custom `read_sse_chunk` helper) hangs in the pglite-oxide test environment: the embedded server's broadcast stream does not produce updates within the test timeout, so subsequent `body.frame()` polls return `Pending` until the deadline fires. These tests cover `account_balance_stream_emits_initial_loading_placeholder`, `account_balance_stream_emits_initial_value_when_state_present`, `account_balance_stream_emits_updates_when_state_changes`, `open_positions_stream_emits_initial_loading_placeholder`, `open_positions_stream_emits_initial_rows_when_state_present`, `open_orders_stream_emits_initial_loading_placeholder`, and `open_orders_stream_emits_initial_rows_when_state_present`. They were never actually executing before the test-DB refactor (the old `DATABASE_URL` gate returned `None` in this environment). Run them with `cargo test -- --ignored` to investigate; fix when the SSE reader can be made to surface initial events without depending on broadcasts from the orchestrator.
