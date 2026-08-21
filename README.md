# HyperVibes

[![Website](https://img.shields.io/badge/Website-hypervibes.ai-0f766e?logo=googlechrome&logoColor=white)](https://hypervibes.ai) [![Documentation](https://img.shields.io/badge/Documentation-2563eb?logo=readthedocs&logoColor=white)](https://hypervibes.ai/docs/) [![Discord](https://img.shields.io/badge/Discord-5865f2?logo=discord&logoColor=white)](https://discord.gg/Up39Qvqmkh) [![X](https://img.shields.io/badge/X-000000?logo=x&logoColor=white)](https://x.com/hypervibes.ai)

HyperVibes is an agent framework for trading on Hyperliquid.

## Quickstart

The production stack runs HyperVibes, its OpenCode runtime, and Postgres in
containers. It publishes HyperVibes only on `127.0.0.1:3003`; it is not
reachable from the network without a separately configured reverse proxy.

Install Podman, `podman-compose`, `curl`, `openssl`, and GNU `sed`, then run
these commands in a new installation directory.

1. Download the Compose file from `master`.

   ```sh
   umask 077 && curl -fsSLo podman-compose.yaml https://raw.githubusercontent.com/idft/hypervibes/master/podman-compose.yaml
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

## Development

Development services, including the disposable test database, are in
`podman-compose.dev.yaml`:

```sh
podman-compose -f podman-compose.dev.yaml up -d
```

Run the application on the host with `cargo run`. See `.env.example` for its
local configuration and `docs/Testing.md` for test database details.

## License

MIT
