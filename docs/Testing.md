
### Test database

By default `cargo test` uses the dedicated `test-postgres` service from `podman-compose.yaml` via `TEST_DATABASE_URL=postgres://vibetrading:vibetrading@127.0.0.1:15433/postgres` from `.cargo/config.toml`.

`src/test_db.rs` creates a fresh database for each `pool()` call when `TEST_DATABASE_URL` is set, runs migrations inside that database, and lets those isolated test databases be created concurrently on the dedicated test Postgres service.

The dedicated test Postgres service is intentionally speed-optimized and disposable:

- port `15433`
- `max_connections=400`
- `fsync=off`
- `synchronous_commit=off`
- `full_page_writes=off`

### Frontend build during tests

`build.rs` skips the automatic frontend `pnpm build` path during `cargo test`. Template and route tests render directly from source templates and do not need compiled assets. If you explicitly need the build-script asset step during a test invocation, run with `VIBETRADING_FORCE_FRONTEND_BUILD=1 cargo test`.

### SSE tests

The SSE route tests in `src/web/routes.rs` run in the normal suite against the dedicated `test-postgres` service. The shared `read_sse_chunk` helper still only reads an initial slice of each long-lived stream, so keep those tests focused on the initial event payload and explicit follow-up updates triggered inside the test.
