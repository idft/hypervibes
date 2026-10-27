import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const repository = fileURLToPath(new URL("../", import.meta.url));

// This fixture runs in a separate process outside the repository. All application
// imports resolve into the npm installation, not the development dependency tree.
const fixture = `
import assert from "node:assert/strict";
import { readFile, readdir, stat, access } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { DatabaseSync } from "node:sqlite";
import net from "node:net";

// No model, exchange, or other network calls are permitted in this test.
net.Socket.prototype.connect = function () { throw new Error("Network access is forbidden in the package test"); };
globalThis.fetch = async () => { throw new Error("Network access is forbidden in the package test"); };
const root = process.argv[2];
const { initializeConfig, defaultConfigPath } = await import(pathToFileURL(join(root, "dist/initialize-config.js")));
const { createApp } = await import(pathToFileURL(join(root, "dist/web/app.js")));
const configPath = defaultConfigPath(process.env);
let previousSecrets;
if (process.argv[3] === "fresh") await assert.rejects(access(configPath));
else previousSecrets = await readFile(configPath, "utf8");
const first = initializeConfig();
const secrets = await readFile(configPath, "utf8");
if (previousSecrets !== undefined) assert.equal(secrets, previousSecrets);
if (process.platform !== "win32") assert.equal((await stat(configPath)).mode & 0o777, 0o600);
assert.equal(first.buildDir, join(root, ".build"));
assert.equal(first.staticDir, join(root, "static"));
let migrationCount;
for (let boot = 0; boot < 2; boot++) {
  const config = initializeConfig();
  assert.equal(config.authSecret, first.authSecret);
  assert.deepEqual(config.encryptionKey, first.encryptionKey);
  assert.equal(await readFile(configPath, "utf8"), secrets);
  const app = await createApp(config);
  try {
    assert.equal(app.server.listening, false);
    const request = (url) => app.inject({ url, headers: { host: "localhost:3003" } });
    const health = await request("/healthz");
    assert.equal(health.statusCode, 200, health.body);
    assert.deepEqual(health.json(), { status: "ok" });
    const login = await request("/login");
    assert.equal(login.statusCode, 200, login.body);
    assert.match(login.headers["content-type"], /text\\/html/);
    assert.match(login.body, /<html/);
    assert.equal((await request("/static/favicon.ico")).statusCode, 200);
    const assets = await readdir(join(config.buildDir, "client/assets"));
    for (const extension of [".js", ".css"]) {
      const asset = assets.find((name) => name.endsWith(extension));
      assert.ok(asset);
      assert.equal((await request("/assets/" + asset)).statusCode, 200);
    }
    for (const name of ["adx", "atr", "bollinger", "ema", "macd", "rsi", "sma"]) {
      assert.ok((await readFile(join(root, "dist/indicators/builtins", name + ".pine"), "utf8")).length);
    }
  } finally {
    await app.shutdown();
  }
  // Read application-owned migration history only, after graceful shutdown.
  const db = new DatabaseSync(config.databasePath, { readOnly: true });
  try {
    const count = Number(db.prepare("SELECT count(*) AS count FROM hv_schema_migrations").get().count);
    assert.ok(count > 0);
    if (boot === 0) migrationCount = count;
    else assert.equal(count, migrationCount);
    assert.ok(db.prepare("SELECT name FROM sqlite_schema WHERE name = 'hv_agents'").get());
  } finally { db.close(); }
  await assert.rejects(access(config.databasePath + ".lock"));
}
console.log(JSON.stringify({ boots: 2, migrations: migrationCount, listening: false, secretsReused: true }));
`;

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: "utf8", timeout: 240_000, maxBuffer: 10 * 1024 * 1024, ...options });
  assert.equal(result.error, undefined, result.error?.message);
  assert.equal(result.status, 0, `${command} ${args.join(" ")}\n${result.stdout}\n${result.stderr}`);
  return result.stdout;
}

test("real npm tarball runs with runtime dependencies outside the repository", { timeout: 240_000 }, async () => {
  const directory = await mkdtemp(join(tmpdir(), "hypervibes-package-"));
  try {
    const working = join(directory, "unrelated");
    const installation = join(directory, "installation");
    await mkdir(working);
    await mkdir(installation);
    const env = {
      PATH: process.env.PATH,
      HOME: join(directory, "home"),
      USERPROFILE: join(directory, "home"),
      APPDATA: join(directory, "config"),
      LOCALAPPDATA: join(directory, "cache"),
      XDG_CONFIG_HOME: join(directory, "config"),
      XDG_DATA_HOME: join(directory, "data"),
      XDG_CACHE_HOME: join(directory, "cache"),
      APP_SQLITE_PATH: join(directory, "data/app.sqlite"),
      APP_CACHE_DIR: join(directory, "cache"),
      npm_config_cache: join(directory, "npm-cache"),
      npm_config_userconfig: join(directory, "npmrc"),
      npm_config_globalconfig: join(directory, "global-npmrc"),
      NODE_ENV: "production",
    };
    // An explicit tarball is useful for diagnostics. The normal command always
    // creates a fresh release, including the prepack build.
    let tarball = process.env.HYPERVIBES_TEST_TARBALL;
    if (!tarball) {
      run("pnpm", ["pack", "--pack-destination", directory], { cwd: repository });
      tarball = join(directory, (await readdir(directory)).find((name) => name.endsWith(".tgz")));
    }
    const contents = run("tar", ["-tzf", tarball]).trim().split("\n");
    for (const path of contents) {
      assert.match(path, /^package\/(?:bin\/|dist\/|\.build\/(?:client|server)\/|static\/|scripts\/check-node\.mjs$|package\.json$|README\.md$|LICENSE$)/, path);
      assert.doesNotMatch(path, /(?:\.env(?:\.|$)|\.sqlite|\.db$|\.test\.|\.map$|\.ts$|memory-review|pi\.md|config\.json$)/, path);
    }
    assert.ok(contents.some((path) => /dist\/db\/migrations\/.+\/migration\.sql$/.test(path)));
    assert.ok(contents.includes("package/.build/server/server.js"));
    await writeFile(join(installation, "package.json"), JSON.stringify({ private: true }));
    run("npm", ["install", "--omit=dev", "--ignore-scripts", "--no-audit", "--no-fund", tarball], { cwd: installation, env });
    const root = join(installation, "node_modules/hypervibes");
    for (const dependency of ["tsx", "typescript"]) {
      await assert.rejects(access(join(installation, "node_modules", dependency)));
    }
    const cli = join(installation, "node_modules/.bin/hypervibes");
    assert.match(run(process.execPath, [cli, "--help"], { cwd: working, env }), /Usage: hypervibes/);
    const metadata = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
    assert.equal(run(process.execPath, [cli, "--version"], { cwd: working, env }).trim(), metadata.version);
    await assert.rejects(access(join(directory, "config/hypervibes/config.json")));
    await assert.rejects(access(env.APP_SQLITE_PATH));
    const script = join(working, "exercise.mjs");
    await writeFile(script, fixture);
    for (const phase of ["fresh", "restart"]) {
      console.log(phase, run(process.execPath, [script, root, phase], { cwd: working, env }).trim());
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
