# HyperVibes

HyperVibes is a framework for AI agents to trade crypto on Hyperliquid exchange.

## Current Work: Incremental TypeScript Rewrite

The active application is a TypeScript rewrite built incrementally alongside the
legacy Rust code. See `pi.md` for design research and history; use the root
`package.json` for current commands.

* Backend TypeScript files live alongside the existing `.rs` files in `src/`.
  Use descriptive TypeScript filenames (e.g. `database.ts`, `app.ts`), not Rust
  module names like `mod.ts`; filenames do not need to match the Rust files.
* Use the existing root `package.json` and pnpm; do not create a separate backend package.
* Keep the Rust/PostgreSQL application intact as a reference. Do not port its
  entire schema or import its existing data.
* Do not run or modify the legacy Rust application as part of rewrite work.

## Application architecture (current)

* **Host:** Fastify (`src/web/app.ts`) owns the process, serving the TanStack
  Start SSR build, static assets, auth, and API routes. Importing modules must
  not start servers or open databases; `src/main.ts` owns listening/shutdown.
* **Frontend:** TanStack Start + React, built with Vite (`vite.app.config.ts`,
  output in `.build/`). Routes live in `src/routes/`; the generated route tree is
  `src/routeTree.gen.ts` (`pnpm generate:routes`).
* **Auth:** better-auth with Drizzle adapter (`src/auth-options.ts`,
  `src/db/auth-schema.ts`).
* **Database:** single SQLite file via Node's built-in `node:sqlite`, opened by
  `src/db/database.ts` (WAL, `synchronous = FULL`, foreign keys on). All access
  is serialized through the `SharedDatabase` queue (`src/db/shared-database.ts`),
  shared by the application (Drizzle over `sqlite-proxy`) and every Pi Durable
  harness. Migrations live in `src/db/migrations/` and are applied by the owning
  runtime at startup (`src/db/migrations.ts`) — never by drizzle-kit directly.
* **Agent execution:** Pi Durable (`@earendil-works/pi-durable`) is integrated.
  Each application agent owns a harness using per-agent table prefixes
  (`pi_<agentid>_…`). Application code must never read or write Pi's private
  tables directly; use the application services (`src/agents/`, `src/harness/`)
  and Pi's public APIs. Coding agents may inspect those tables read-only for
  diagnostic debugging of development runs, scoped to the relevant agent/run.
  This diagnostic exception does not permit writes or application dependencies
  on Pi's internal schema.
* **Domain features implemented so far:** per-user provider connections and model
  catalog (`src/providers/`, `src/model_catalog/`), agent dashboard/chat with
  history and compaction, configurable sub-agents with schedules and delegation
  (`src/db/subagent-schema.ts`), Hyperliquid account streaming and order/account
  tools (`src/hyperliquid/`), and the trade journal with account-history sync.

## Development database

The dev database is a single SQLite file (no PostgreSQL, no containers):

* **Location:** `APP_SQLITE_PATH` overrides the OS-dependent default. Consult
  `defaultDatabasePath` in `src/config.ts` for the authoritative resolution:
  * Linux/other Unix: `$XDG_DATA_HOME/hypervibes/hypervibes.sqlite` when
    `XDG_DATA_HOME` is absolute; otherwise
    `$HOME/.local/share/hypervibes/hypervibes.sqlite`.
  * macOS: `$HOME/Library/Application Support/HyperVibes/hypervibes.sqlite`.
  * Windows: `%APPDATA%\HyperVibes\hypervibes.sqlite` when `APPDATA` is absolute;
    otherwise `<home>\AppData\Roaming\HyperVibes\hypervibes.sqlite`.
  The dev server loads `.env`; relative path overrides resolve from the repo
  root. The schema is created/migrated on server startup.
* **Read-only inspection with the sqlite3 CLI is fine while the server runs**
  (the database is in WAL mode, so concurrent readers are safe):
  ```sh
  # Set DB to the resolved database file path described above.
  sqlite3 -readonly "$DB" '.tables'
  sqlite3 -readonly "$DB" 'select * from hv_agents;'
  ```
* Application tables are prefixed `hv_`; better-auth owns its user/session tables.
  Agent harness tables use per-agent `pi_..._` prefixes (see `harnessPrefix` in
  `src/agents/agent-input.ts`). Coding agents may inspect them read-only for
  development diagnostics after checking the installed Pi storage schema and
  scoping queries to the relevant agent/run. Never modify them by hand.
* Never run write statements against the live dev database from scripts, tests,
  or ad-hoc commands. Tests must use disposable SQLite files (`src/test-config.ts`).
* If the server is not running, the file is still readable, but prefer the
  `-readonly` flag regardless. A lock file alongside the resolved database may
  mean the app is running or did not shut down cleanly; never remove it as part
  of inspection.

## Legacy Rust application (reference only)

* Rust, axum, sqlx, PostgreSQL, HTMX/SSE, podman-compose.
* Do not run `cargo` commands, Rust tests, or `just test` for TypeScript work;
  they can hang waiting for PostgreSQL. Only on explicit user request.
* Never stop the running podman-compose stack.

## Running and building

* Backend dev server: `pnpm dev:server` (port 3003, loads `.env`).
  **Never start the dev webserver yourself; the user will run it.**
* Frontend build: `pnpm build:app` (Vite/TanStack Start). Don't test web UI
  changes with browser-mcp; the user will test.
* Buttons should change the cursor on mouseover.

## Testing

* Run backend checks from the repo root: `pnpm typecheck:server`,
  `pnpm lint:server`, and `pnpm test:server`.
* Backend tests use disposable SQLite files and injected HTTP requests, without
  opening a listening port or depending on PostgreSQL, containers, or model calls.
* For frontend changes, use `pnpm typecheck`, `pnpm lint`, and `pnpm test:js` as
  appropriate.
* Never point tests at the running application's database or use real exchange
  credentials.
* Use a 240 second timeout for tests.

## Coding agent rules

* Don't guess. Consult the documentation or source code if you are unsure.
* Never `git push` without permission.
* Do not create database migrations without permission, unless a plan
  specifically mentions creating it.

## Documentation during the rewrite

* **Do not create or update documentation for TypeScript rewrite work unless
  the user explicitly requests it.** This includes `docs/`, README files, and
  `pi.md`.
* Keep the existing public documentation for the legacy application unchanged
  while the rewrite is in progress.
* Maintain `AGENTS.md` when the user requests changes to coding-agent instructions.

## Code Quality & Linting Standards

* Use strict TypeScript types and keep backend Node configuration separate from
  browser configuration.
* Keep application construction separate from listening; importing modules must
  not start servers, open databases, or launch agent work.
* Close owned resources on shutdown and on initialization failure.
* Remove unreachable, unused, or unreferenced code. Do not add speculative
  feature scaffolding or suppress diagnostics without a documented reason.
* Preserve explicit error handling and domain/security boundaries. Do not use
  floating-point numbers as financial authority.
* Keep application schema ownership separate from Pi Durable's internal storage;
  do not write its private tables directly.

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