import { randomBytes } from "node:crypto";
import { constants, closeSync, fchmodSync, fsyncSync, linkSync, lstatSync, mkdirSync, openSync, readFileSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, win32 } from "node:path";
import { loadConfig, type AppConfig } from "./config.js";

/** Persistent secrets are configuration, not disposable cache or database contents. */
export function defaultConfigPath(env: NodeJS.ProcessEnv, platform = process.platform, home = homedir()): string {
  if (platform === "win32") {
    const base = env.APPDATA?.trim();
    return win32.join(base && win32.isAbsolute(base) ? base : win32.join(home, "AppData", "Roaming"), "HyperVibes", "config.json");
  }
  if (platform === "darwin") return join(home, "Library", "Application Support", "HyperVibes", "config.json");
  const base = env.XDG_CONFIG_HOME?.trim();
  return join(base && isAbsolute(base) ? base : join(home, ".config"), "hypervibes", "config.json");
}

interface StoredSecrets {
  version: 1;
  BETTER_AUTH_SECRET: string;
  AGENTS_ENCRYPTION_KEY: string;
}

function missing(error: unknown): boolean {
  return error instanceof Error && "code" in error && error.code === "ENOENT";
}

function effectiveConfig(secrets: StoredSecrets, env: NodeJS.ProcessEnv): AppConfig {
  const config = loadConfig({ ...env, BETTER_AUTH_SECRET: env.BETTER_AUTH_SECRET ?? secrets.BETTER_AUTH_SECRET, AGENTS_ENCRYPTION_KEY: env.AGENTS_ENCRYPTION_KEY ?? secrets.AGENTS_ENCRYPTION_KEY });
  if (!config.encryptionKey) throw new Error("AGENTS_ENCRYPTION_KEY must not be empty");
  return config;
}

function protectDirectory(directory: string): void {
  if (!lstatSync(directory).isDirectory()) throw new Error("The private configuration directory must not be a symbolic link");
  if (process.platform === "win32") return; // Node modes do not manage Windows ACLs; use the user's profile ACL.
  const descriptor = openSync(directory, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try { fchmodSync(descriptor, 0o700); }
  finally { closeSync(descriptor); }
}

function readSecrets(path: string): StoredSecrets | undefined {
  let descriptor: number;
  try {
    if (!lstatSync(path).isFile()) throw new Error("The private configuration must be a regular file");
    descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  } catch (error) {
    if (missing(error)) return undefined;
    throw error;
  }
  try {
    if (process.platform !== "win32") fchmodSync(descriptor, 0o600);
    let value: unknown;
    try { value = JSON.parse(readFileSync(descriptor, "utf8")); }
    catch { throw new Error("Invalid private configuration JSON; restore the original configuration or supply the original secrets"); }
    if (!value || typeof value !== "object" || !("version" in value) || value.version !== 1
      || !("BETTER_AUTH_SECRET" in value) || typeof value.BETTER_AUTH_SECRET !== "string"
      || !("AGENTS_ENCRYPTION_KEY" in value) || typeof value.AGENTS_ENCRYPTION_KEY !== "string"
      || !value.AGENTS_ENCRYPTION_KEY.trim()) {
      throw new Error("Invalid or incomplete private configuration; restore the original configuration");
    }
    loadConfig({ BETTER_AUTH_SECRET: value.BETTER_AUTH_SECRET, AGENTS_ENCRYPTION_KEY: value.AGENTS_ENCRYPTION_KEY });
    return value as StoredSecrets;
  } finally { closeSync(descriptor); }
}

/** Called by the executable, never during imports. Does not open or modify SQLite. */
export function initializeConfig(env: NodeJS.ProcessEnv = process.env, path = defaultConfigPath(env)): AppConfig {
  let stored: StoredSecrets | undefined;
  try {
    try { protectDirectory(dirname(path)); }
    catch (error) { if (!missing(error)) throw error; }
    stored = readSecrets(path);
  } catch (error) {
    // Complete external secrets can recover service without modifying damaged configuration.
    if (env.BETTER_AUTH_SECRET !== undefined && env.AGENTS_ENCRYPTION_KEY !== undefined) {
      return effectiveConfig({ version: 1, BETTER_AUTH_SECRET: env.BETTER_AUTH_SECRET, AGENTS_ENCRYPTION_KEY: env.AGENTS_ENCRYPTION_KEY }, env);
    }
    throw error;
  }
  if (stored) return effectiveConfig(stored, env);

  // Validate supplied settings before creating files. Blank/invalid overrides must fail.
  const candidate: StoredSecrets = {
    version: 1,
    BETTER_AUTH_SECRET: env.BETTER_AUTH_SECRET ?? randomBytes(32).toString("hex"),
    AGENTS_ENCRYPTION_KEY: env.AGENTS_ENCRYPTION_KEY ?? randomBytes(32).toString("hex"),
  };
  const config = effectiveConfig(candidate, env);
  if (env.AGENTS_ENCRYPTION_KEY === undefined) {
    // Conservatively protect every existing profile, including unknown/older schemas.
    // We cannot infer whether an absent key previously encrypted credentials.
    for (const file of [config.databasePath, `${config.databasePath}-wal`, `${config.databasePath}.lock`]) {
      try {
        statSync(file);
        throw new Error("Existing database without a saved encryption key: restore the original config.json or supply the original AGENTS_ENCRYPTION_KEY. No replacement key was saved.");
      } catch (error) { if (!missing(error)) throw error; }
    }
  }

  const directory = dirname(path);
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  protectDirectory(directory);
  const directoryDescriptor = process.platform === "win32" ? undefined : openSync(directory, constants.O_RDONLY);
  let temporary: string | undefined;
  try {
    if (directoryDescriptor !== undefined) fchmodSync(directoryDescriptor, 0o700);
    const temporaryPath = join(directory, `.config-${randomBytes(16).toString("hex")}.tmp`);
    const descriptor = openSync(temporaryPath, "wx", 0o600);
    temporary = temporaryPath;
    try {
      writeFileSync(descriptor, `${JSON.stringify(candidate, null, 2)}\n`);
      fsyncSync(descriptor);
    } finally { closeSync(descriptor); }
    try {
      // A hard link publishes complete contents without replacing a concurrent winner.
      linkSync(temporary, path);
      if (directoryDescriptor !== undefined) fsyncSync(directoryDescriptor);
    } catch (error) {
      if (!(error instanceof Error && "code" in error && error.code === "EEXIST")) throw error;
    }
  } finally {
    try { if (temporary) unlinkSync(temporary); }
    finally { if (directoryDescriptor !== undefined) closeSync(directoryDescriptor); }
  }
  const winner = readSecrets(path);
  if (!winner) throw new Error("Private configuration disappeared during initialization");
  return effectiveConfig(winner, env);
}
