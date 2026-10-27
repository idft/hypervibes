---
slug: /installation
---

# Installation

HyperVibes can run from source. Release packages are not available yet.

## From source

Install Node.js 22.23.3 or newer and pnpm. In the source checkout, run:

```sh
pnpm install
pnpm start
```

Keep development dependencies installed: startup builds fresh production assets.
Open <http://localhost:3003>. No `.env` file is required. The first start creates
private secrets in your per-user `config.json`. Back up that file with your
database. Keep existing encryption keys when upgrading.

For another device, prefer HTTPS and set `APP_ORIGIN` to its exact browser URL.
Private LAN HTTP requires these optional `.env` settings (replace the IP):

```dotenv
APP_HOST=0.0.0.0
APP_ORIGIN=http://10.16.20.111:3003
APP_ALLOW_INSECURE_HTTP=true
```

Use that exact URL. HTTP exposes sessions and data to the network; use it only
on a trusted private network. Some browser wallets require HTTPS.

Continue with [Quick start](QuickStart.md) after opening the app.

## NPX

NPX will be the preferred way to run HyperVibes on your own computer.

Coming soon. The package name, required Node.js version, and launch command
will be added when the package is ready.

## Desktop app

Desktop builds are planned for:

- macOS
- Windows
- Linux

Coming soon. Downloads and installation steps are not available yet.

## Server hosting

A Raspberry Pi or a virtual private server may be useful if you want agents
to run while your personal computer is off. Server installation steps and
Raspberry Pi compatibility checks are still to do.

For any always-on installation:

- Keep the host running and connected to the internet for scheduled work and Telegram.
- Keep access private. Do not expose the app directly to the public internet.
- Back up your app data and encryption secrets together. Both are needed to recover stored credentials.
- Protect the host as you would a device with access to your trading account.

Release package instructions will be added when available.
