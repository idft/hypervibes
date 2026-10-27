import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
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
const progress = (message) => console.error('[fixture ' + process.argv[3] + '] ' + message);
progress('import configuration');
const { initializeConfig, defaultConfigPath } = await import(pathToFileURL(join(root, "dist/initialize-config.js")));
const configPath = defaultConfigPath(process.env);
if (process.argv[3] === "inspect") {
  await assert.rejects(access(configPath), { code: "ENOENT" });
  console.log(JSON.stringify(configPath));
} else {
const stallDiagnostic = setTimeout(() => progress('still running; active resources: ' + process.getActiveResourcesInfo().join(', ')), 20_000);
stallDiagnostic.unref();
progress('import application');
const { createApp } = await import(pathToFileURL(join(root, "dist/web/app.js")));
let previousSecrets;
if (process.argv[3] === "fresh") await assert.rejects(access(configPath));
else previousSecrets = await readFile(configPath, "utf8");
progress('initialize configuration');
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
  progress('boot ' + boot + ': create application');
  const app = await createApp(config);
  progress('boot ' + boot + ': application ready');
  try {
    assert.equal(app.server.listening, false);
    const request = (url) => app.inject({ url, headers: { host: "localhost:3003" } });
    const health = await request("/healthz");
    assert.equal(health.statusCode, 200, health.body);
    assert.deepEqual(health.json(), { status: "ok" });
    progress('boot ' + boot + ': request login SSR');
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
    progress('boot ' + boot + ': shutdown');
    await app.shutdown();
    progress('boot ' + boot + ': shutdown complete');
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
progress('checks complete; waiting for process exit');
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

async function run(command, args, options = {}) {
  const { label = command, timeoutMs = 15_000, stream = false, signal, ...spawnOptions } = options;
  const env = spawnOptions.env ?? process.env;
  const executable = process.platform === "win32" ? resolveWindowsBatch(command, env) : command;
  const target = invocation(executable, args, process.platform, env);
  signal?.throwIfAborted();
  console.error(`[package] START ${label} (limit ${timeoutMs / 1000}s)`);
  const started = Date.now();
  return await new Promise((resolve, reject) => {
    const child = spawn(target.command, target.args, {
      ...spawnOptions, stdio: ["ignore", "pipe", "pipe"],
      detached: process.platform !== "win32",
      windowsVerbatimArguments: target.windowsVerbatimArguments,
    });
    let stdout = "", stderr = "", failure, teardownTimer;
    let settled = false;
    const finish = (code, exitSignal) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      clearTimeout(teardownTimer);
      signal?.removeEventListener("abort", aborted);
      if (failure || code !== 0) {
        reject(new Error(`${label}: ${failure?.message ?? `exit ${code}, signal ${exitSignal}`}\n${stdout}\n${stderr}`));
      } else {
        console.error(`[package] PASS ${label} (${((Date.now() - started) / 1000).toFixed(1)}s)`);
        resolve(stdout);
      }
    };
    const stop = (reason) => {
      if (failure) return;
      failure = reason;
      teardownTimer = setTimeout(() => {
        failure = new Error(`${reason.message}; process-tree cleanup did not close output pipes within 2s`);
        child.stdout.destroy();
        child.stderr.destroy();
        child.unref();
        finish(null, null);
      }, 2_000);
      if (!child.pid) return;
      // cmd.exe can leave npm's Node child alive. Kill only this test's process
      // tree. Bound the wait if an orphan keeps the output pipes open.
      if (process.platform === "win32") {
        const root = Object.entries(env).find(([key]) => key.toLowerCase() === "systemroot")?.[1];
        const killer = spawnSync(root ? win32.join(root, "System32", "taskkill.exe") : "taskkill.exe",
          ["/pid", String(child.pid), "/T", "/F"], { timeout: 5_000, windowsHide: true });
        if (killer.error || killer.status !== 0) {
          stderr += `\ntaskkill failed (status ${killer.status}): ${killer.error?.message ?? ""}\n${killer.stdout ?? ""}\n${killer.stderr ?? ""}`;
          child.kill("SIGKILL");
        }
      } else {
        try { process.kill(-child.pid, "SIGKILL"); }
        catch { child.kill("SIGKILL"); }
      }
    };
    const aborted = () => stop(new Error(`${label} aborted`));
    const timer = setTimeout(() => stop(new Error(`${label} timed out after ${timeoutMs / 1000}s`)), timeoutMs);
    signal?.addEventListener("abort", aborted, { once: true });
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (data) => {
      stdout = (stdout + data).slice(-1024 * 1024);
      if (stream) console.error(`[${label}] ${data.trimEnd()}`);
    });
    child.stderr.on("data", (data) => {
      stderr = (stderr + data).slice(-1024 * 1024);
      if (stream) console.error(`[${label}] ${data.trimEnd()}`);
    });
    child.on("error", (error) => { failure ??= error; });
    child.on("close", finish);
  });
}

test("command runner captures output and reports failure and timeout phases", async () => {
  assert.equal(await run(process.execPath, ["-e", 'console.log("ready")'], { label: "runner success" }), "ready\n");
  await assert.rejects(run(process.execPath, ["-e", 'console.error("fixture failure"); process.exitCode = 7'], {
    label: "runner failure",
  }), /runner failure: exit 7[\s\S]*fixture failure/);
  await assert.rejects(run(process.execPath, ["--input-type=module", "-e", `
    import { spawn } from "node:child_process";
    spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "inherit" });
    console.log("waiting for timeout");
    setInterval(() => {}, 1000);
  `], { label: "runner process-tree timeout", timeoutMs: 2000 }), /runner process-tree timeout timed out after 2s[\s\S]*waiting for timeout/);
});

test("command runner aborts an active command", async () => {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 300);
  try {
    await assert.rejects(run(process.execPath, ["-e", "setInterval(() => {}, 1000)"], {
      label: "runner abort", signal: controller.signal,
    }), /runner abort aborted/);
  } finally { clearTimeout(timer); }
});

test("command runner bounds cleanup when an exited parent leaves an orphan holding pipes", { timeout: 15_000 }, async () => {
  let failure, output = "";
  const started = Date.now();
  try {
    output = await run(process.execPath, ["--input-type=module", "-e", `
      import { spawn } from "node:child_process";
      const orphan = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { detached: true, stdio: "inherit" });
      console.log("DESCENDANT_PID=" + orphan.pid);
      orphan.unref();
      process.exit(0);
    `], { label: "runner orphan timeout", timeoutMs: 1500 });
    assert.fail("An orphan holding output pipes must not report success");
  } catch (error) {
    failure = error;
  } finally {
    // This fixture deliberately escapes the parent's process group. Remove
    // only the child PID printed by our fixture after testing bounded failure.
    const pid = /DESCENDANT_PID=(\d+)/.exec((failure?.message ?? "") + output)?.[1];
    if (pid) {
      try { process.kill(Number(pid), "SIGKILL"); }
      catch (error) { if (error.code !== "ESRCH") throw error; }
    }
  }
  assert.match(failure.message, /runner orphan timeout timed out[\s\S]*cleanup did not close output pipes/);
  assert.ok(Date.now() - started < 12_000, "Cleanup must not wait indefinitely for pipe closure");
});

test("Windows wrapper preserves its directory and arguments during execution", { skip: process.platform !== "win32" }, async () => {
  const directory = await realpath(await mkdtemp(join(tmpdir(), "hypervibes-batch-")));
  try {
    const wrapperDirectory = join(directory, "space & (data)");
    await mkdir(wrapperDirectory);
    const wrapper = join(wrapperDirectory, "fixture.cmd");
    await writeFile(wrapper, '@echo off\r\necho "%~dp0"\r\necho "%~1"\r\n');
    const output = await run(wrapper, ["argument & (value)"], { cwd: directory, label: "Windows wrapper execution" });
    const lines = output.trim().split(/\r?\n/);
    assert.equal(lines[0], `"${wrapperDirectory}\\"`);
    assert.equal(lines[1], '"argument & (value)"');
  } finally {
    await rm(directory, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
});

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

test("real npm tarball runs with runtime dependencies outside the repository", { timeout: 240_000 }, async (context) => {
  const execute = (command, args, options) => run(command, args, { signal: context.signal, ...options });
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
      await execute("pnpm", ["pack", "--pack-destination", directory], { cwd: repository, label: "build release tarball", timeoutMs: 90_000, stream: true });
      tarball = join(directory, (await readdir(directory)).find((name) => name.endsWith(".tgz")));
    }
    const contents = archiveMembers(await execute("tar", ["-tzf", tarball], { label: "inspect tarball" }));
    for (const path of contents) {
      assert.match(path, /^package\/(?:bin\/|dist\/|\.build\/(?:client|server)\/|static\/|scripts\/check-node\.mjs$|package\.json$|README\.md$|LICENSE$)/, path);
      assert.doesNotMatch(path, /(?:\.env(?:\.|$)|\.sqlite|\.db$|\.test\.|\.map$|\.ts$|memory-review|pi\.md|config\.json$)/, path);
    }
    assert.ok(contents.some((path) => /dist\/db\/migrations\/.+\/migration\.sql$/.test(path)));
    assert.ok(contents.includes("package/.build/server/server.js"));
    await writeFile(join(installation, "package.json"), JSON.stringify({ private: true }));
    await execute("npm", ["install", "--omit=dev", "--ignore-scripts", "--no-audit", "--no-fund",
      "--loglevel=http", "--fetch-retries=1", "--fetch-timeout=30000", tarball], {
      cwd: installation, env, label: "install runtime dependencies", timeoutMs: 90_000, stream: true,
    });
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
    const configPath = JSON.parse(await execute(process.execPath, [script, root, "inspect"], {
      cwd: working, env: appEnv, label: "inspect isolated configuration", stream: true,
    }));
    const configRelative = relative(directory, configPath);
    assert.ok(configRelative && !isAbsolute(configRelative) && configRelative.split(/[\\/]/)[0] !== "..", "Configuration must stay in the temporary directory");
    // Execute the actual npm shim: a symlink on POSIX, a batch file on Windows.
    // A Windows .bin/hypervibes shell script is not JavaScript for node to run.
    const cli = join(installation, "node_modules/.bin/hypervibes" + (process.platform === "win32" ? ".cmd" : ""));
    assert.match(await execute(cli, ["--help"], { cwd: working, env: appEnv, label: "CLI help", stream: true }), /Usage: hypervibes/);
    const metadata = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
    assert.equal((await execute(cli, ["--version"], { cwd: working, env: appEnv, label: "CLI version", stream: true })).trim(), metadata.version);
    await assert.rejects(access(configPath), { code: "ENOENT" });
    await assert.rejects(access(env.APP_SQLITE_PATH), { code: "ENOENT" });
    for (const phase of ["fresh", "restart"]) {
      console.log(phase, (await execute(process.execPath, [script, root, phase], {
        cwd: working, env: appEnv, label: `${phase} application process`, timeoutMs: 30_000, stream: true,
      })).trim());
    }
  } finally {
    console.error("[package] remove disposable installation and profile");
    await rm(directory, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
    console.error("[package] cleanup complete");
  }
});
