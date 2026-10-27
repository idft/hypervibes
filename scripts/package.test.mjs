import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { access, mkdir, mkdtemp, readFile, readdir, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, dirname, isAbsolute, join, relative, win32 } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { test } from "node:test";

const repository = fileURLToPath(new URL("../", import.meta.url));

// This fixture runs in a separate process outside the repository. All application
// imports resolve into the npm installation, not the development dependency tree.
const fixture = `
import assert from "node:assert/strict";
import { readFile, readdir, realpath, stat, access } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { DatabaseSync } from "node:sqlite";

const root = process.argv[2];
const { initializeConfig, defaultConfigPath } = await import(pathToFileURL(join(root, "dist/initialize-config.js")));
const configPath = defaultConfigPath(process.env);
if (process.argv[3] === "inspect") {
  await assert.rejects(access(configPath), { code: "ENOENT" });
  console.log(JSON.stringify(configPath));
} else {
const { createApp } = await import(pathToFileURL(join(root, "dist/web/app.js")));
let previousSecrets;
if (process.argv[3] === "fresh") await assert.rejects(access(configPath));
else previousSecrets = await readFile(configPath, "utf8");
const first = initializeConfig();
const secrets = await readFile(configPath, "utf8");
if (previousSecrets !== undefined) assert.equal(secrets, previousSecrets);
if (process.platform !== "win32") assert.equal((await stat(configPath)).mode & 0o777, 0o600);
assert.equal(await realpath(first.buildDir), await realpath(join(root, ".build")));
assert.equal(await realpath(first.staticDir), await realpath(join(root, "static")));
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
}
`;

// HOME controls os.homedir() on macOS; XDG_CONFIG_HOME does not control its
// configuration path. Check the actual child home before any application import.
const guard = `
import assert from "node:assert/strict";
import { homedir } from "node:os";
import net from "node:net";
assert.equal(homedir(), process.env.HOME, "The child home must be isolated");
net.Server.prototype.listen = function () { throw new Error("Listeners are forbidden in the package test"); };
net.Socket.prototype.connect = function () { throw new Error("Network access is forbidden in the package test"); };
globalThis.fetch = async () => { throw new Error("Network access is forbidden in the package test"); };
`;

function invocation(command, args, platform = process.platform, env = process.env) {
  if (platform !== "win32") return { command, args };
  if (["npm", "pnpm"].includes(command)) command += ".cmd";
  if (!command.toLowerCase().endsWith(".cmd")) return { command, args };
  // Batch files require cmd.exe, not shell:true with an interpolated tarball.
  // Quote every token. Fail closed for characters that cmd can expand even in
  // quotes, or that could end a quoted token. Never pass them to a shell.
  const quote = (value) => {
    assert.doesNotMatch(value, /["%!\r\n\0]/, "Unsafe Windows batch argument");
    return `"${value}"`;
  };
  const comspec = Object.entries(env).find(([key]) => key.toLowerCase() === "comspec")?.[1];
  return {
    command: comspec || "cmd.exe",
    args: ["/d", "/s", "/v:off", "/c", `"${[command, ...args].map(quote).join(" ")}"`],
    windowsVerbatimArguments: true,
  };
}

function resolveWindowsBatch(command, env, exists = existsSync) {
  if (["npm", "pnpm"].includes(command)) command += ".cmd";
  if (!command.toLowerCase().endsWith(".cmd") || win32.isAbsolute(command)) return command;
  const path = Object.entries(env).find(([key]) => key.toLowerCase() === "path")?.[1] ?? "";
  for (const directory of path.split(";")) {
    const candidate = win32.join(directory.replace(/^"|"$/g, ""), command);
    if (win32.isAbsolute(candidate) && exists(candidate)) return candidate;
  }
  throw new Error(`Cannot resolve Windows batch command on PATH: ${command}`);
}

function archiveMembers(output) {
  return output.trim().split(/\r?\n/);
}

function run(command, args, options = {}) {
  const env = options.env ?? process.env;
  // A bare batch filename can make npm's %~dp0 point at the test working
  // directory. Resolve the wrapper before cmd.exe runs it, preserving its base.
  const executable = process.platform === "win32" ? resolveWindowsBatch(command, env) : command;
  const target = invocation(executable, args, process.platform, env);
  const result = spawnSync(target.command, target.args, {
    encoding: "utf8", timeout: 240_000, maxBuffer: 10 * 1024 * 1024,
    ...options, windowsVerbatimArguments: target.windowsVerbatimArguments,
  });
  assert.equal(result.error, undefined, result.error?.message);
  assert.equal(result.status, 0, `${command} ${args.join(" ")}\n${result.stdout}\n${result.stderr}`);
  return result.stdout;
}

test("Windows batch commands quote paths and reject shell expansion", () => {
  const target = invocation("npm", ["install", "C:\\test space & (data)\\package.tgz"], "win32", { ComSpec: "C:\\Windows\\System32\\cmd.exe" });
  assert.equal(target.command, "C:\\Windows\\System32\\cmd.exe");
  assert.deepEqual(target.args, ["/d", "/s", "/v:off", "/c", '""npm.cmd" "install" "C:\\test space & (data)\\package.tgz""']);
  assert.equal(target.windowsVerbatimArguments, true);
  assert.equal(invocation("pnpm", ["pack"], "win32", {}).args.at(-1), '""pnpm.cmd" "pack""');
  for (const unsafe of ['a" & echo bad', "%PATH%", "!PATH!", "a\nb", "a\rb", "a\0b"]) {
    assert.throws(() => invocation("npm", [unsafe], "win32", {}), /Unsafe Windows batch argument/);
  }
  assert.deepEqual(invocation(process.execPath, ["file.mjs"], "win32"), { command: process.execPath, args: ["file.mjs"] });
  assert.deepEqual(invocation("npm", ["install"], "linux"), { command: "npm", args: ["install"] });
});

test("Windows batch commands resolve from PATH before invocation", () => {
  const wrapper = "C:\\Program Files\\nodejs\\npm.cmd";
  const env = { Path: 'C:\\missing;"C:\\Program Files\\nodejs"' };
  assert.equal(resolveWindowsBatch("npm", env, (candidate) => candidate === wrapper), wrapper);
  assert.equal(resolveWindowsBatch(wrapper, {}), wrapper);
  assert.equal(resolveWindowsBatch("C:\\tools\\hypervibes.cmd", {}), "C:\\tools\\hypervibes.cmd");
  assert.equal(resolveWindowsBatch("node.exe", {}), "node.exe");
  assert.throws(() => resolveWindowsBatch("npm", env, () => false), /Cannot resolve/);
  assert.throws(() => resolveWindowsBatch("npm", { PATH: "." }, () => true), /Cannot resolve/);
  const target = invocation(resolveWindowsBatch("npm", env, (candidate) => candidate === wrapper), ["install"], "win32", {});
  assert.equal(target.args.at(-1), '""C:\\Program Files\\nodejs\\npm.cmd" "install""');
});

test("resource paths compare equal through a directory alias", async () => {
  const directory = await realpath(await mkdtemp(join(tmpdir(), "hypervibes-paths-")));
  try {
    const original = join(directory, "original");
    const alias = join(directory, "alias");
    await mkdir(original);
    await mkdir(join(original, ".build"));
    await symlink(original, alias, process.platform === "win32" ? "junction" : "dir");
    assert.notEqual(join(original, ".build"), join(alias, ".build"));
    assert.equal(await realpath(join(original, ".build")), await realpath(join(alias, ".build")));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("archive members have no carriage returns with LF or CRLF output", () => {
  const expected = ["package/package.json", "package/.build/server/server.js"];
  for (const newline of ["\n", "\r\n"]) {
    assert.deepEqual(archiveMembers(expected.join(newline) + newline), expected);
  }
});

test("real npm tarball runs with runtime dependencies outside the repository", { timeout: 240_000 }, async () => {
  // macOS exposes /var through /private/var. Use one canonical temporary root
  // for isolation checks and Node's realpath-resolved module locations.
  const directory = await realpath(await mkdtemp(join(tmpdir(), "hypervibes-package-")));
  try {
    const working = join(directory, "unrelated");
    const installation = join(directory, "installation");
    await mkdir(working);
    await mkdir(installation);
    // Keep only operating-system essentials, without inheriting app secrets,
    // NODE_OPTIONS, or the real user's npm configuration and profile paths.
    const inherited = {};
    for (const name of ["PATH", "SystemRoot", "SystemDrive", "WINDIR", "ComSpec", "PATHEXT"]) {
      const value = Object.entries(process.env).find(([key]) => key.toLowerCase() === name.toLowerCase())?.[1];
      if (value !== undefined) inherited[name] = value;
    }
    const env = {
      ...inherited,
      PATH: [dirname(process.execPath), inherited.PATH].filter(Boolean).join(delimiter),
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
      TMPDIR: join(directory, "tmp"),
      TMP: join(directory, "tmp"),
      TEMP: join(directory, "tmp"),
      NODE_ENV: "production",
    };
    await mkdir(env.HOME);
    await mkdir(env.TMPDIR);
    // An explicit tarball is useful for diagnostics. The normal command always
    // creates a fresh release, including the prepack build.
    let tarball = process.env.HYPERVIBES_TEST_TARBALL;
    if (tarball) assert.ok(isAbsolute(tarball), "HYPERVIBES_TEST_TARBALL must be absolute");
    if (!tarball) {
      run("pnpm", ["pack", "--pack-destination", directory], { cwd: repository });
      tarball = join(directory, (await readdir(directory)).find((name) => name.endsWith(".tgz")));
    }
    const contents = archiveMembers(run("tar", ["-tzf", tarball]));
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
    const guardPath = join(working, "guard.mjs");
    await writeFile(guardPath, guard);
    const appEnv = { ...env, NODE_OPTIONS: `--import=${pathToFileURL(guardPath).href}` };
    const script = join(working, "exercise.mjs");
    await writeFile(script, fixture);
    // Ask the installed code for its platform-specific path before initialization.
    const configPath = JSON.parse(run(process.execPath, [script, root, "inspect"], { cwd: working, env: appEnv }));
    const configRelative = relative(directory, configPath);
    assert.ok(configRelative && !isAbsolute(configRelative) && configRelative.split(/[\\/]/)[0] !== "..", "Configuration must stay in the temporary directory");
    // Execute the actual npm shim: a symlink on POSIX, a batch file on Windows.
    // A Windows .bin/hypervibes shell script is not JavaScript for node to run.
    const cli = join(installation, "node_modules/.bin/hypervibes" + (process.platform === "win32" ? ".cmd" : ""));
    assert.match(run(cli, ["--help"], { cwd: working, env: appEnv }), /Usage: hypervibes/);
    const metadata = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
    assert.equal(run(cli, ["--version"], { cwd: working, env: appEnv }).trim(), metadata.version);
    await assert.rejects(access(configPath), { code: "ENOENT" });
    await assert.rejects(access(env.APP_SQLITE_PATH), { code: "ENOENT" });
    for (const phase of ["fresh", "restart"]) {
      console.log(phase, run(process.execPath, [script, root, phase], { cwd: working, env: appEnv }).trim());
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
