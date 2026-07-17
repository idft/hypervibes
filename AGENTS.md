# Vibetrading V2

This is a framework for allowing OpenCode-backed AI agents to trade crypto on Hyperliquid exchange.

See `docs/` for project documentation.
Keep documentation up to date when making signifigant code changes or archetecutre decisions.
You do not need to update the docs for UI changes.

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

* Run `cargo test` from the repo root. Tests use the dedicated `test-postgres` service from `podman-compose.yaml` via `TEST_DATABASE_URL=postgres://vibetrading:vibetrading@127.0.0.1:15433/postgres`. `src/test_db.rs` creates isolated databases for test helper calls against that service and runs migrations in each one so database setup can happen concurrently. The dev server's container Postgres at `localhost:15432` (u: vibetrading, p: vibetrading, db: vibetrading) is never touched by tests.
* Run `cargo fix --allow-dirty` to properly format files

## Coding agent Rules:

* Don't guess. Consult the documentation or source code if you are unsure.
* Never `git push` without permission
* Do not create database migrations without permission, unless a plan specficailly mentions creating it.

## Code Quality & Linting Standards

Strict Clippy Enforcement: All Rust code must pass cargo clippy --workspace --all-targets -- -D warnings before commit. This treats dead code warnings as compilation errors, forcing immediate resolution rather than suppression. 

Dead Code Policy:

Remove unreachable, unused, or unreferenced code immediately
Never use #[allow(dead_code)] unless documenting a specific exception (e.g., public API stability, future feature scaffolding)
Document any allowed dead code with a comment explaining why it exists and when it will be addressed 
Error Handling Requirements:

Use ? operator for error propagation in library code
Replace .unwrap() with .expect("descriptive message") only for invariant violations
Use thiserror for library errors and anyhow for application entry points 
Type System Leverage:

Maximize compile-time safety through Rust's type system
Avoid unwrap() in library code; prefer explicit error handling
Make match statements exhaustive; avoid wildcard arms when possible 
Pre-Commit Automation:

### Run before every commit
```
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace
```

Definition of Done: No new Clippy warnings, no dead code, all tests passing.
