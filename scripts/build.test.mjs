import assert from "node:assert/strict";
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { loadConfig } from "../dist/config.js";
import { createApp } from "../dist/web/app.js";

test("compiled application loads migrations, bundled indicators, SSR, and static assets without listening", async () => {
  const directory = await mkdtemp(join(tmpdir(), "hypervibes-build-"));
  let app;
  try {
    const config = loadConfig({
      APP_SQLITE_PATH: join(directory, "app.sqlite"),
      APP_CACHE_DIR: join(directory, "cache"),
      BETTER_AUTH_SECRET: "disposable-build-test-secret-not-for-production",
      AGENTS_ENCRYPTION_KEY: "ab".repeat(32),
    });
    app = await createApp(config);
    assert.equal(app.server.listening, false);
    const health = await app.inject({ url: "/healthz", headers: { host: "localhost:3003" } });
    assert.equal(health.statusCode, 200, health.body);
    assert.deepEqual(health.json(), { status: "ok" });
    const login = await app.inject({ url: "/login", headers: { host: "localhost:3003" } });
    assert.equal(login.statusCode, 200, login.body);
    assert.match(login.headers["content-type"], /text\/html/);
    const icon = await app.inject({ url: "/static/favicon.ico", headers: { host: "localhost:3003" } });
    assert.equal(icon.statusCode, 200);
    const assets = await readdir(join(config.buildDir, "client", "assets"));
    const asset = assets.find((name) => name.endsWith(".js"));
    assert.ok(asset, "client JavaScript must be built");
    assert.equal((await app.inject({ url: `/assets/${asset}`, headers: { host: "localhost:3003" } })).statusCode, 200);
    for (const name of ["adx", "atr", "bollinger", "ema", "macd", "rsi", "sma"]) {
      assert.ok((await readFile(new URL(`../dist/indicators/builtins/${name}.pine`, import.meta.url), "utf8")).length);
    }
  } finally {
    try { await app?.close(); }
    finally { await rm(directory, { recursive: true, force: true }); }
  }
});
