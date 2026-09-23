---
slug: /installation
---

# Installation

Podman Compose is currently the recommended installation method. The Compose
file runs three containers: the HyperVibes application server and web UI, a
PostgreSQL database, and an OpenCode instance.

Linux is currently the only supported platform. macOS and Windows may be
supported in the future, but for now we recommend installing HyperVibes in its
own separate Linux virtual machine.

## Podman Compose install (recommended)

Install Podman, `podman-compose`, `curl`, and `openssl`. Create a
private installation directory, then download the current Compose file from the
`master` branch.

```sh
umask 077 && curl -fsSLo podman-compose.yaml https://raw.githubusercontent.com/idft/hypervibes/master/podman-compose.yaml
```

Generate unique secrets in a private `.env` file before the first start. This
command refuses to overwrite an existing `.env`:

```sh
(umask 077; set -C; KEY=$(openssl rand -hex 32) && CTRL=$(openssl rand -hex 32) && OC=$(openssl rand -hex 32) && DB=$(openssl rand -hex 32) && printf 'AGENTS_ENCRYPTION_KEY=%s\nWORKSPACE_CONTROL_API_KEY=%s\nOPENCODE_SERVER_PASSWORD=%s\nPOSTGRES_PASSWORD=%s\n' "$KEY" "$CTRL" "$OC" "$DB" > .env)
```

From that directory, start the stack (`podman-compose` reads `.env` for Compose
variable substitution):

```sh
podman-compose up -d
```

When the services are healthy, open [http://127.0.0.1:3003](http://127.0.0.1:3003).
Continue with [Quick Start](/docs/quick-start).

Keep `.env` private and back it up with the database: it contains the
agent-encryption key and database password. Do not regenerate it after the first
start. `podman-compose down` preserves data volumes; do not use
`podman-compose down -v` unless you intend to delete the database, provider
credentials, and generated workspaces.

## Run from source

Install Rust, Node.js 20 or later, `pnpm`, Podman, and `podman-compose`. Start
the development services, including the dedicated test database, from the
repository root:

```sh
podman-compose -f podman-compose.dev.yaml up -d
```

Configure the local application using `.env.example`, then run HyperVibes from
the repository root:

```sh
cargo run
```

Indicator execution normally derives its worker count from available logical
CPUs. Set `INDICATOR_MAX_CONCURRENT_EXECUTIONS` to `1..=32` to impose a
per-process limit, and optionally set
`INDICATOR_ANALYSIS_WAIT_TIMEOUT_SECONDS` to `1..=300` (default `30`).

The development application uses the same local web address,
[http://127.0.0.1:3003](http://127.0.0.1:3003), unless its configuration changes
the bind address or port. See [Testing](Testing.md) for the test database and
test commands.
