# Vibetrading V2

This is a framework for allowing AI Agents (such as Hermes) to trade crypto on Hyperliquid exchange.

See `docs/` for project documentation.
Keep documentation up to date when making signifigant code changes or archetecutre decisions.
You do not need to update the docs for UI changes.

The server (`cargo run`) may be running in another tmux page, for reading logs and restarting the server
(Make sure it is in the same directory as this project, and not another very-similar project!)

## Stack

* Rust
* axum
* sqlx
* postgres
* HTMX, SSE for live updates

## Frontend build

Frontend assets are managed with `pnpm` and built via `esbuild` and Tailwind CSS v4:

- Source files: `assets/app.ts`, `assets/app.css`
- Built output: `static/dist/app.js`, `static/dist/app.css` (gitignored)
- `build.rs` runs `pnpm install --frozen-lockfile` and `pnpm build` automatically during `cargo build`/`cargo run` when sources or templates change.

* Don't manually build the front-end assets.
* Don't test web UI changes with browser-mcp.  The user will test.
* Web server defaults to running on port 3003
* Never start the dev webserver yourself.  The user will run it.
* Never stop the running podman-compose

## Testing

Run `cargo test` from the repo root. No `podman compose`, no Docker, no external Postgres is required: DB-touching tests use an embedded `pglite-oxide` PostgreSQL server that starts in-process via `src/test_db.rs`. The dev server's container Postgres at `localhost:15432` (u: vibetrading, p: vibetrading) is never touched by tests.

## Hermes Agent Profile Distribution

The Hermes Agent profile distribution is Located in `../vibetrading-hermes-profile`.  This is a  hermes profile that can be installed by any user of Hermes Agent

## Coding agent Rules:

* Don't guess. Consult the documentation or source code if you are unsure.
* Never `git push` without permission
* Do not create database migrations without permission, unless a plan specficailly mentions creating it.

## Future plans

The Hermes agent backend system has been deprecated.  When planning features, do not worry about implementing them for the Hermes sytsem.
