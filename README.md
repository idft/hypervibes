# HyperVibes

[![Website](https://img.shields.io/badge/Website-hypervibes.ai-0f766e?logo=googlechrome&logoColor=white)](https://hypervibes.ai) [![Documentation](https://img.shields.io/badge/Documentation-2563eb?logo=readthedocs&logoColor=white)](https://hypervibes.ai/docs/) [![Discord](https://img.shields.io/badge/Discord-5865f2?logo=discord&logoColor=white)](https://discord.gg/Up39Qvqmkh) [![X](https://img.shields.io/badge/X-000000?logo=x&logoColor=white)](https://x.com/HyperVibesAI)

HyperVibes is a self-hosted app for running AI trading agents on Hyperliquid.
Choose your models, give agents instructions, and review their work in one place.

Agents can place real orders. AI can make mistakes, and trading can lose money.
Start with trading disabled while you check your setup.

## Run from source

HyperVibes requires Node.js 22.23.3 or newer and pnpm.

1. Install dependencies.

   ```sh
   pnpm install
   ```

2. Build and start the app. No `.env` file is required.

   ```sh
   pnpm start
   ```

Each start builds fresh production assets before serving. Install development
dependencies too: the source build needs them. Use `pnpm dev` instead
for development with hot reload. `pnpm start:built` serves an existing compiled
build without rebuilding.

Open <http://localhost:3003>. The SQLite database is created
automatically on first start. Authentication and credential-encryption secrets are
generated once and saved privately outside the project:

- Linux: `$XDG_CONFIG_HOME/hypervibes/config.json`, or `~/.config/hypervibes/config.json`.
- macOS: `~/Library/Application Support/HyperVibes/config.json`.
- Windows: `%APPDATA%\HyperVibes\config.json`, or the home `AppData\Roaming` equivalent.

Back up this file with your database. Losing the encryption key prevents recovery
of stored credentials. Existing installations without this file must supply their
original `AGENTS_ENCRYPTION_KEY`; startup never silently replaces a missing key.

Environment variables override saved settings. Later secret overrides are not
saved, so back them up too. An optional `.env` file is loaded by the development
and server commands; see `.env.example`. Keep existing secrets when upgrading.
Disposable caches use your OS user-cache directory; `APP_CACHE_DIR` overrides it.

For access from another device, prefer HTTPS through a reverse proxy and set
`APP_ORIGIN` to the exact browser URL. Private LAN HTTP requires an explicit
opt-in in `.env`, for example:

```dotenv
APP_HOST=0.0.0.0
APP_ORIGIN=http://10.16.20.111:3003
APP_ALLOW_INSECURE_HTTP=true
```

Open that exact URL, not `localhost` or another hostname. HTTP sends sessions
and app data without transport encryption. Use it only on a trusted private
network, never directly on the public internet. Some browser wallets require
HTTPS and may not work over LAN HTTP.

See `docs/` for the user guide and `AGENTS.md` for architecture and project
conventions.

## Build a release package

```sh
pnpm build          # Compiled backend, frontend, and runtime resources
pnpm start:built    # Run that build without rebuilding
pnpm pack           # Rebuild and create a local npm tarball
```

The package exposes the `hypervibes` executable and runs with Node.js and runtime
dependencies only; it does not need pnpm or TypeScript to launch. Run
`node bin/hypervibes.js --help` for usage. Public npm releases are coming soon.

## Checks

```sh
pnpm typecheck:server
pnpm lint:server
pnpm test:server
pnpm test:startup
pnpm build
pnpm test:build
pnpm test:package
```

GitHub Actions runs validation on pull requests, pushes to `master` and `v*`
branches, and manual runs. It builds one npm tarball and tests that artifact on
Linux, macOS, and Windows with runtime dependencies only. The workflow uploads
the tarball for inspection; it does not publish a release or package.

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
