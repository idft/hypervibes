# HyperVibes

[![Website](https://img.shields.io/badge/Website-hypervibes.ai-0f766e?logo=googlechrome&logoColor=white)](https://hypervibes.ai) [![Documentation](https://img.shields.io/badge/Documentation-2563eb?logo=readthedocs&logoColor=white)](https://hypervibes.ai/docs/) [![Discord](https://img.shields.io/badge/Discord-5865f2?logo=discord&logoColor=white)](https://discord.gg/Up39Qvqmkh) [![X](https://img.shields.io/badge/X-000000?logo=x&logoColor=white)](https://x.com/HyperVibesAI)

HyperVibes is a self-hosted app for running AI trading agents on Hyperliquid.
Choose your models, give agents instructions, and review their work in one place.

Agents can place real orders. AI can make mistakes, and trading can lose money.
Start with trading disabled while you check your setup.

## Development

HyperVibes requires Node.js 22.23.3 or newer and pnpm.

1. Install dependencies.

   ```sh
   pnpm install
   ```

2. Create local configuration and set a random auth secret.

   ```sh
   cp .env.example .env
   openssl rand -hex 32
   ```

   Paste the generated value into `BETTER_AUTH_SECRET` in `.env`.

3. Start the development server.

   ```sh
   pnpm dev:server
   ```

Open <http://localhost:3003>. The SQLite development database is created
automatically on first start.

See `docs/` for the user guide and `AGENTS.md` for architecture and project
conventions.

## Checks

```sh
pnpm typecheck:server
pnpm lint:server
pnpm test:server
```

## License

HyperVibes is licensed under the [GNU Affero General Public License v3.0 only](LICENSE)
(`AGPL-3.0-only`).

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
