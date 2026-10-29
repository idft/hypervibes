---
slug: /installation
---

# Installation

HyperVibes runs from the published npm package, a container image, or source.

## NPX

NPX is the quickest way to run HyperVibes on your own computer. Install
Node.js 22.23.3 or newer, then run:

```sh
npx hypervibes
```

Open [http://localhost:3003](http://localhost:3003). The first start creates
private secrets in your per-user `config.json`. Back up that file with your
database. Keep existing encryption keys when upgrading.

To install it once instead, run `npm install -g hypervibes` and start it
with `hypervibes`.

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

## From source

Install Node.js 22.23.3 or newer and pnpm. In the source checkout, run:

```sh
pnpm install
pnpm start
```

Keep development dependencies installed: startup builds fresh production assets.

## Containers

The Linux AMD64 and ARM64 image is available at
`ghcr.io/idft/hypervibes:0.4.0`. Earlier images use the legacy application and
are not interchangeable with this setup.

Use `compose.yaml` from this repository:

```sh
docker compose up -d
```

Open [http://localhost:3003](http://localhost:3003). The example exposes only the host loopback address.
For remote access, use a private connection or an HTTPS reverse proxy and set
`APP_ORIGIN` to the exact browser URL. The app listens on `0.0.0.0` inside the
container; this does not require exposing the host port publicly.

The named volume holds both the SQLite database and generated secrets:

- Database: `/var/lib/hypervibes/data/hypervibes/hypervibes.sqlite`
- Secrets: `/var/lib/hypervibes/config/hypervibes/config.json`

Back up the complete volume with the app stopped. Keep it when replacing the
container; never use `docker compose down -v` unless you intend to delete all
app data and secrets. Losing the encryption key prevents recovery of saved
credentials. Restore the original secrets with an existing database, not newly
generated replacements. Bind mounts must be writable by container UID/GID 1000.
The `/tmp` cache is disposable. Shutdown waits for running agent work; the
example allows 31 minutes before the container runtime forces termination.

To upgrade, back up the volume, update the image version in `compose.yaml`, and
run `docker compose pull && docker compose up -d`.

## Desktop app

Desktop builds are planned for:

- macOS
- Windows
- Linux

Downloads and installation steps are not available.

## Server hosting

A Raspberry Pi or a virtual private server may be useful if you want agents
to run while your personal computer is off. Server installation steps and
Raspberry Pi hardware compatibility checks are still to do.

For any always-on installation:

- Keep the host running and connected to the internet for scheduled work and Telegram.
- Keep access private. Do not expose the app directly to the public internet.
- Back up your app data and encryption secrets together. Both are needed to recover stored credentials.
- Protect the host as you would a device with access to your trading account.

Public release downloads will be linked when available.
