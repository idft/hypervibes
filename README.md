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

## Disclaimer

HyperVibes is open-source software that can enable agents to submit orders to
Hyperliquid using user-authorized signing keys. It is not financial,
investment, trading, legal, or tax advice, and is not a broker, dealer,
exchange, custodian, or financial intermediary.

Trading digital assets and derivatives involves substantial risk, including the
possible loss of all capital. AI-generated analysis, automated strategies, and
software outputs may be incorrect or unsuitable for any purpose. You are solely
responsible for your trading decisions, agent configuration, credentials,
monitoring, and compliance with applicable laws and regulations.

The software and related materials are provided "as is" without warranties of
any kind. To the maximum extent permitted by law, the authors and copyright
holders are not liable for losses, damages, or expenses arising from use of the
software, including trading losses, software defects, configuration errors,
service outages, or third-party failures.
