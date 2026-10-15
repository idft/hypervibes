import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export interface AppConfig {
  host: string;
  port: number;
  databasePath: string;
  staticDir: string;
  buildDir: string;
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

  return {
    host: setting(env, "APP_HOST", "127.0.0.1"),
    port: Number(port),
    databasePath: resolve(setting(env, "APP_SQLITE_PATH", "data/hypervibes.sqlite")),
    buildDir: resolve(setting(env, "APP_BUILD_DIR", fileURLToPath(new URL("../.build/", import.meta.url)))),
    staticDir: resolve(setting(
      env,
      "APP_STATIC_DIR",
      fileURLToPath(new URL("../static/", import.meta.url)),
    )),
  };
}
