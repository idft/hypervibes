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
- `curl`, `openssl`, and GNU `sed`
- A browser wallet for Hyperliquid login and account approvals
- An OpenCode provider connection before an agent can run sub-agents

## Install

Create a new installation directory, then download the current Compose file from
the `master` branch.

```sh
umask 077 && curl -fsSLo podman-compose.yaml https://raw.githubusercontent.com/idft/hypervibes/master/podman-compose.yaml
```

Generate unique secrets before the first start:

```sh
KEY=$(openssl rand -hex 32) && CTRL=$(openssl rand -hex 32) && OC=$(openssl rand -hex 32) && DB=$(openssl rand -hex 32) && sed -i -e "s/REPLACE_KEY/$KEY/" -e "s/REPLACE_CTRL/$CTRL/" -e "s/REPLACE_OC/$OC/" -e "s/REPLACE_DB/$DB/" podman-compose.yaml
```

Start the stack:

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

`podman-compose.yaml` contains generated secrets. Keep it private and back it
up with the database. Do not run the secret-generation command against an
existing installation. `podman-compose down` preserves data volumes; do not use
`podman-compose down -v` unless you intend to delete the database, provider
credentials, and generated workspaces.
