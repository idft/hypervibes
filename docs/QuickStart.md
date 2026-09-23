---
slug: /quick-start
---

# Quick Start

HyperVibes runs as a local container stack with the HyperVibes application,
OpenCode runtime, and Postgres. It is intended to be operated by the account
owner; do not expose the application to a public network without first adding
your own access control and TLS termination.

## Before you begin

You need:

- Podman and `podman-compose`
- `curl` and `openssl`
- A browser wallet for Hyperliquid login and account approvals
- An OpenCode provider connection before an agent can run sub-agents

## Install

Create a new installation directory, then download the current Compose file from
the `master` branch.

```sh
umask 077 && curl -fsSLo podman-compose.yaml https://raw.githubusercontent.com/idft/hypervibes/master/podman-compose.yaml
```

Generate unique secrets in a private `.env` file before the first start. This
command refuses to overwrite an existing `.env`:

```sh
(umask 077; set -C; KEY=$(openssl rand -hex 32) && CTRL=$(openssl rand -hex 32) && OC=$(openssl rand -hex 32) && DB=$(openssl rand -hex 32) && printf 'AGENTS_ENCRYPTION_KEY=%s\nWORKSPACE_CONTROL_API_KEY=%s\nOPENCODE_SERVER_PASSWORD=%s\nPOSTGRES_PASSWORD=%s\n' "$KEY" "$CTRL" "$OC" "$DB" > .env)
```

From that directory, start the stack:

```sh
podman-compose up -d
```

When the services are healthy, open [http://127.0.0.1:3003](http://127.0.0.1:3003).

## Set up an agent

1. Sign in with your browser wallet.
2. On the Account page, create or import and approve a Hyperliquid trading
   signer.
3. Connect an LLM provider in Provider connections.
4. Create an agent and assign one main or sub-account, a market allowlist,
   prompts, and sub-agent models.
5. Review its setup state before enabling scheduled work.

Read [Hyperliquid configuration](Hyperliquid.md) and [Agents](Agents.md) before
enabling an agent that can place live orders.

## Keep the installation safe

`.env` contains generated secrets. Keep it private and back it up with the
database. Do not regenerate it after the first start. `podman-compose down`
preserves data volumes; do not use `podman-compose down -v`
unless you intend to delete the database, provider credentials, and generated
workspaces.
