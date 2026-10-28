---
slug: /configuration
---

# Configuration

HyperVibes works without an `.env` file. On first start, it creates a private
`config.json` containing authentication and credential-encryption secrets.
Server and storage settings use environment variables, not `config.json`.
Restart the app after changing settings.

## Files and directories

| OS | `hypervibes.sqlite` directory | `config.json` directory | Disposable cache directory |
| --- | --- | --- | --- |
| Linux | `~/.local/share/hypervibes/` | `~/.config/hypervibes/` | `~/.cache/hypervibes/` |
| macOS | `~/Library/Application Support/HyperVibes/` | Same as database | `~/Library/Caches/HyperVibes/` |
| Windows | `%APPDATA%\HyperVibes\` | Same as database | `%LOCALAPPDATA%\HyperVibes\Cache\` |

On Linux, absolute `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, and `XDG_CACHE_HOME`
values replace the respective base directories. On Windows, missing or relative
`APPDATA` and `LOCALAPPDATA` values fall back to the home directory's
`AppData\Roaming` and `AppData\Local` directories.

`APP_SQLITE_PATH` overrides the database filename; `APP_CACHE_DIR` overrides
the cache directory. There is no environment variable for a custom config filename.
For container paths and backups, see [Installation](Installation.md#containers).

## Config file

`config.json` supports these fields only. Other keys do not configure the app.
The app generates the secrets automatically; you normally do not need to edit them.

| Field | Value |
| --- | --- |
| `version` | Required number `1`. |
| `BETTER_AUTH_SECRET` | Authentication secret containing at least 32 random characters. |
| `AGENTS_ENCRYPTION_KEY` | Credential-encryption key: exactly 64 hexadecimal characters (32 bytes). |

Back up the database and `config.json` together. Losing the encryption key
prevents recovery of stored credentials. Keep the original secrets when upgrading
or moving an installation. Do not replace the encryption key to reset access.

An existing database without saved configuration requires the original
`AGENTS_ENCRYPTION_KEY` in the environment; startup does not silently generate a
replacement. First-run environment secrets are saved in the new config file.
Later environment overrides take precedence but do not update the saved file,
so back up those overrides separately.

## Environment variables

Set variables in your shell, container environment, or an optional `.env` file.
The source launch commands and packaged executable load `.env` from the current
working directory. Existing shell variables take precedence over `.env` values.
Relative storage and asset paths resolve from the working directory.

| Variable | Default | Purpose |
| --- | --- | --- |
| `APP_HOST` | `127.0.0.1` | Address the server binds to. Use `0.0.0.0` to listen on all IPv4 interfaces. |
| `APP_PORT` | `3003` | Server port, from `1` to `65535`. |
| `APP_ORIGIN` | `http://localhost:<APP_PORT>` | Exact browser origin for authentication. Must be HTTP(S), without credentials, a path, query, or fragment. |
| `APP_ALLOW_INSECURE_HTTP` | `false` | Set to `true` to allow non-loopback HTTP origins on a trusted private network. Only `true` and `false` are accepted. |
| `APP_SQLITE_PATH` | OS database path above | SQLite database filename. |
| `APP_CACHE_DIR` | OS cache directory above | Disposable cached assets. |
| `APP_STATIC_DIR` | Application's `static/` directory | Static asset directory. |
| `APP_BUILD_DIR` | Application's `.build/` directory | Built frontend directory. |
| `BETTER_AUTH_SECRET` | Saved config value | Override the authentication secret. At least 32 random characters. |
| `AGENTS_ENCRYPTION_KEY` | Saved config value | Override the credential-encryption key. Exactly 64 hexadecimal characters. |
| `XDG_DATA_HOME` | `~/.local/share` | Linux/Unix database base directory; must be absolute to take effect. |
| `XDG_CONFIG_HOME` | `~/.config` | Linux/Unix config base directory; must be absolute to take effect. |
| `XDG_CACHE_HOME` | `~/.cache` | Linux/Unix cache base directory; must be absolute to take effect. |
| `APPDATA` | Home directory's `AppData\Roaming` | Windows database and config base directory; must be absolute to take effect. |
| `LOCALAPPDATA` | Home directory's `AppData\Local` | Windows cache base directory; must be absolute to take effect. |

Do not use blank secret overrides. See the repository's [`.env.example`](https://github.com/idft/hypervibes/blob/master/.env.example)
for a starting point.

## Access from another device

Prefer HTTPS through a reverse proxy and set `APP_ORIGIN` to the exact browser URL.
For private LAN HTTP, use settings like these in `.env`, replacing the IP address:

```dotenv
APP_HOST=0.0.0.0
APP_ORIGIN=http://10.16.20.111:3003
APP_ALLOW_INSECURE_HTTP=true
```

Open that exact URL. HTTP sends sessions and app data without transport encryption.
Use it only on a trusted private network, never directly on the public internet.
Some browser wallets require HTTPS and may not work over LAN HTTP.
