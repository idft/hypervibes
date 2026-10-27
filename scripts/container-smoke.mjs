// Run inside the production image. No listener, credentials, or external network.
import assert from "node:assert/strict";
import net from "node:net";
import { readFile, readdir, access, writeFile } from "node:fs/promises";
import { join } from "node:path";

assert.notEqual(process.getuid(), 0, "The runtime must not be root");
net.Server.prototype.listen = function () { throw new Error("Listeners are forbidden"); };
net.Socket.prototype.connect = function () { throw new Error("Network access is forbidden"); };
globalThis.fetch = async () => { throw new Error("Network access is forbidden"); };
const root = "/opt/hypervibes/node_modules/hypervibes";
const { initializeConfig, defaultConfigPath } = await import(`${root}/dist/initialize-config.js`);
const { createApp } = await import(`${root}/dist/web/app.js`);
const config = initializeConfig();
const secrets = await readFile(defaultConfigPath(process.env), "utf8");
const snapshot = "/var/lib/hypervibes/test-secret-snapshot";
try {
  assert.equal(await readFile(snapshot, "utf8"), secrets, "Container replacement must reuse secrets");
} catch (error) {
  if (error.code !== "ENOENT") throw error;
  await writeFile(snapshot, secrets, { mode: 0o600, flag: "wx" });
}
assert.equal(config.host, "0.0.0.0");
assert.equal(config.databasePath, "/var/lib/hypervibes/data/hypervibes/hypervibes.sqlite");
assert.equal(defaultConfigPath(process.env), "/var/lib/hypervibes/config/hypervibes/config.json");
for (let boot = 0; boot < 2; boot++) {
  const current = initializeConfig();
  assert.equal(await readFile(defaultConfigPath(process.env), "utf8"), secrets);
  assert.deepEqual(current.encryptionKey, config.encryptionKey);
  const app = await createApp(current);
  try {
    assert.equal(app.server.listening, false);
    for (const url of ["/healthz", "/login", "/static/favicon.ico"]) {
      const response = await app.inject({ url, headers: { host: "localhost:3003" } });
      assert.equal(response.statusCode, 200, response.body);
    }
    const assets = await readdir(join(current.buildDir, "client/assets"));
    for (const extension of [".js", ".css"]) {
      const asset = assets.find((name) => name.endsWith(extension));
      assert.ok(asset);
      assert.equal((await app.inject({ url: `/assets/${asset}`, headers: { host: "localhost:3003" } })).statusCode, 200);
    }
    assert.ok((await readFile(`${root}/dist/indicators/builtins/rsi.pine`, "utf8")).length);
  } finally {
    await app.shutdown();
  }
  await assert.rejects(access(`${current.databasePath}.lock`));
}
for (const dependency of ["tsx", "typescript"]) {
  await assert.rejects(access(`/opt/hypervibes/node_modules/${dependency}`));
}
console.log("Container smoke passed: non-root, 2 boots, persistent secrets, migrations, SSR, assets, no listener or network");
