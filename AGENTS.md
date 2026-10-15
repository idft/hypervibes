# HyperVibes

HyperVibes is a framework for AI agents to trade crypto on Hyperliquid exchange.

## Current Work: Incremental TypeScript Rewrite

We are building a new TypeScript application from scratch, implementing features
incrementally rather than converting the entire Rust app. See `pi.md` for existing
design research; use the root `package.json` for current commands.

* Write new backend TypeScript files directly alongside the existing `.rs` files in `src/`. Use descriptive TypeScript filenames (e.g. `database.ts`, `app.ts`), not Rust module names like `mod.ts`; filenames do not need to match the Rust files.
* Use the existing root `package.json` and pnpm; do not create a separate backend package.
* The scaffold uses Fastify and SQLite via Node's built-in `node:sqlite`.
* Pi Durable integration is deferred. Later, one harness can manage conversations linked to application agent records; starting the scaffold must not start agents or make model calls.
* Keep the existing UI, templates, and frontend build; modify them incrementally as features are implemented.
* Keep the Rust/OpenCode/PostgreSQL application intact as a reference. Do not port its entire schema or import its existing data.
* Domain tables, schema migrations, APIs, workers, and trading functionality are future work, not implied by the scaffold.

## Documentation During the Rewrite

* **Do not create or update documentation for TypeScript rewrite work unless the user explicitly requests it.** This includes `docs/`, README files, and `pi.md`.
* Keep the existing public documentation for the legacy application unchanged while the rewrite is in progress.
* Maintain `AGENTS.md` when the user requests changes to coding-agent instructions.

## Stack

New application:
* TypeScript/ESM on Node.js 22.23.3 or a newer supported, patched Node release
* Fastify
* SQLite (`node:sqlite`)
* pnpm
* Pi Durable planned for agent execution

Existing application (reference, not the current implementation target):
* Rust, axum, sqlx, PostgreSQL
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
* Never stop the running podman-compose stack

## Testing

* **Do not run Rust tests (`cargo test`, including workspace variants) while working on the TypeScript rewrite.** They are unrelated to the new app and can hang waiting for PostgreSQL. Only run them if the user explicitly requests it.
* Do not run `just test`: it includes the legacy Rust suite. Do not run Cargo build, Clippy, or formatting checks by default for TypeScript-only work.
* Run backend checks from the repo root: `pnpm typecheck:server`, `pnpm lint:server`, and `pnpm test:server`.
* Backend tests use disposable SQLite files and injected HTTP requests, without opening a listening port or depending on PostgreSQL, containers, or model calls.
* For frontend changes, use `pnpm typecheck`, `pnpm lint`, and `pnpm test:js` as appropriate.
* Never point tests at the running application's database or use real exchange credentials.
* Use a 240 second timeout for tests.

## UI Rules

* Buttons should change the cursor on mouseover

## Coding agent Rules:

* Don't guess. Consult the documentation or source code if you are unsure.
* Never `git push` without permission
* Do not create database migrations without permission, unless a plan specficailly mentions creating it.

## Code Quality & Linting Standards

* Use strict TypeScript types and keep backend Node configuration separate from browser configuration.
* Keep application construction separate from listening; importing modules must not start servers, open databases, or launch agent work.
* Close owned resources on shutdown and on initialization failure.
* Remove unreachable, unused, or unreferenced code. Do not add speculative feature scaffolding or suppress diagnostics without a documented reason.
* Preserve explicit error handling and domain/security boundaries. Do not use floating-point numbers as financial authority.
* Keep application schema ownership separate from Pi Durable's internal storage; do not write its private tables directly.

### Run before commits affecting the TypeScript backend

```sh
pnpm typecheck:server
pnpm lint:server
pnpm test:server
```

Run the relevant frontend checks when changing frontend code. Do not add Rust
checks to the rewrite workflow; legacy Rust testing requires an explicit user request.

Definition of Done: Relevant TypeScript checks pass, with no new diagnostics or
dead code. Documentation updates are not part of the rewrite workflow unless
explicitly requested.
