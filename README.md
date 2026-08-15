# HyperVibes

HyperVibes is an agent framework for trading on Hyperliquid.

## Quickstart

The production stack runs HyperVibes, its OpenCode runtime, and Postgres in
containers. It publishes HyperVibes only on `127.0.0.1:3003`; it is not
reachable from the network without a separately configured reverse proxy.

Install Podman, `podman-compose`, `curl`, `openssl`, and GNU `sed`, then run
these commands in a new installation directory.

1. Download the Compose file for the release you want to run. Replace
   `v0.3.0` with the desired release tag.

   ```sh
   umask 077 && curl -fsSLo podman-compose.yaml https://raw.githubusercontent.com/johnkozan/hypervibes/v0.3.0/podman-compose.yaml
   ```

2. Generate unique local secrets. Run this once, before the first start.

   ```sh
   KEY=$(openssl rand -hex 32) && CTRL=$(openssl rand -hex 32) && OC=$(openssl rand -hex 32) && DB=$(openssl rand -hex 32) && sed -i -e "s/REPLACE_KEY/$KEY/" -e "s/REPLACE_CTRL/$CTRL/" -e "s/REPLACE_OC/$OC/" -e "s/REPLACE_DB/$DB/" podman-compose.yaml
   ```

3. Start the stack.

   ```sh
   podman-compose up -d
   ```

Open <http://127.0.0.1:3003> after the services become healthy.

## Operations

`podman-compose.yaml` contains the deployment's generated secrets. Keep it
private, back it up with the database, and do not run the secret-generation
command against an existing installation. The agent encryption key in that
file is required to decrypt stored agent credentials.

`podman-compose down` preserves data volumes. Do not use `podman-compose down -v`
unless you intentionally want to delete the database, OpenCode provider
credentials, and generated agent workspaces.

To upgrade an existing deployment to the image tag configured in its Compose
file, pull and recreate the services without replacing the Compose file:

```sh
podman-compose pull && podman-compose up -d
```

The first release intentionally does not include a public reverse proxy or TLS
termination. Add one explicitly when remote access is required.

## Development

Development services, including the disposable test database, are in
`podman-compose.dev.yaml`:

```sh
podman-compose -f podman-compose.dev.yaml up -d
```

Run the application on the host with `cargo run`. See `.env.example` for its
local configuration and `docs/Testing.md` for test database details.
