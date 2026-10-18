import { isAbsolute, join, resolve, win32 } from "node:path";
import { homedir, tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

export interface EncryptionKeySetting {
  /** Identifier stored alongside ciphertext so keys can be rotated and audited. */
  id: string;
  bytes: Uint8Array;
}

export interface AppConfig {
  host: string;
  port: number;
  databasePath: string;
  staticDir: string;
  /** Disposable asset storage; hosts such as Electron may supply their cache directory. */
  cacheDir?: string;
  buildDir: string;
  authOrigin: string;
  authSecret: string;
  /** Optional server secret for encrypting exchange and provider credentials. */
  encryptionKey: EncryptionKeySetting | undefined;
}

/** OS-appropriate per-user data directory, so packaged hosts reuse one profile. */
export function defaultDatabasePath(env: NodeJS.ProcessEnv, platform = process.platform, home = homedir()): string {
  if (platform === "win32") {
    const appData = env.APPDATA?.trim();
    return win32.join(appData && win32.isAbsolute(appData) ? appData : win32.join(home, "AppData", "Roaming"), "HyperVibes", "hypervibes.sqlite");
  }
  if (platform === "darwin") return join(home, "Library", "Application Support", "HyperVibes", "hypervibes.sqlite");
  const dataHome = env.XDG_DATA_HOME?.trim();
  return join(dataHome && isAbsolute(dataHome) ? dataHome : join(home, ".local", "share"), "hypervibes", "hypervibes.sqlite");
}

function setting(env: NodeJS.ProcessEnv, name: string, fallback: string): string {
  const value = (env[name] ?? fallback).trim();
  if (!value) throw new Error(`${name} must not be empty`);
  return value;
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): AppConfig {
  const port = setting(env, "APP_PORT", "3003");
  if (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535) {
    throw new Error("APP_PORT must be an integer between 1 and 65535");
  }

  let origin: URL;
  try { origin = new URL(setting(env, "APP_ORIGIN", `http://localhost:${port}`)); }
  catch { throw new Error("APP_ORIGIN must be a valid HTTP(S) origin"); }
  if (origin.username || origin.password || origin.pathname !== "/" || origin.search || origin.hash
    || !["http:", "https:"].includes(origin.protocol)) {
    throw new Error("APP_ORIGIN must be an HTTP(S) origin without credentials, path, query or fragment");
  }
  if (origin.protocol !== "https:" && !["localhost", "127.0.0.1", "[::1]"].includes(origin.hostname)) {
    throw new Error("APP_ORIGIN requires HTTPS except on loopback");
  }
  const authSecret = setting(env, "BETTER_AUTH_SECRET", "");
  if (authSecret.length < 32) throw new Error("BETTER_AUTH_SECRET must contain at least 32 random characters");
  const encryptionKey = encryptionKeySetting(env);

  return {
    authOrigin: origin.origin,
    authSecret,
    host: setting(env, "APP_HOST", "127.0.0.1"),
    port: Number(port),
    databasePath: resolve(setting(env, "APP_SQLITE_PATH", defaultDatabasePath(env))),
    buildDir: resolve(setting(env, "APP_BUILD_DIR", fileURLToPath(new URL("../.build/", import.meta.url)))),
    staticDir: resolve(setting(
      env,
      "APP_STATIC_DIR",
      fileURLToPath(new URL("../static/", import.meta.url)),
    )),
    cacheDir: resolve(setting(env, "APP_CACHE_DIR", join(tmpdir(), "hypervibes"))),
    encryptionKey,
  };
}

/** Parses the optional AGENTS_ENCRYPTION_KEY secret for stored credentials. */
function encryptionKeySetting(env: NodeJS.ProcessEnv): EncryptionKeySetting | undefined {
  const keyHex = (env.AGENTS_ENCRYPTION_KEY ?? "").trim();
  if (!keyHex) return undefined;
  if (!/^(?:[0-9a-fA-F]{2})+$/.test(keyHex)) throw new Error("AGENTS_ENCRYPTION_KEY must be a valid hexadecimal string");
  const bytes = Buffer.from(keyHex, "hex");
  if (bytes.length !== 32) throw new Error(`AGENTS_ENCRYPTION_KEY must decode to exactly 32 bytes, got ${bytes.length}`);
  // Rows record the ID that encrypted them, so a future key-rotation scheme can
  // distinguish versions. Today there is exactly one version.
  return { id: "primary", bytes: new Uint8Array(bytes) };
}
