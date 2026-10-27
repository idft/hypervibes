import { execFile } from "node:child_process";
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { afterEach, describe, expect, it } from "vitest";
import { defaultConfigPath, initializeConfig } from "./initialize-config.js";
import { testAuthSecret } from "./test-config.js";

const directories: string[] = [];
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "hypervibes-config-test-"));
  directories.push(root);
  return { root, path: join(root, "private", "config.json"), env: { APP_SQLITE_PATH: join(root, "profile.sqlite") } };
}
afterEach(() => { for (const directory of directories.splice(0)) rmSync(directory, { recursive: true, force: true }); });

const supplied = { BETTER_AUTH_SECRET: testAuthSecret, AGENTS_ENCRYPTION_KEY: "12".repeat(32) };

describe("first-run configuration", () => {
  it("generates private secrets and reuses them without opening a database", () => {
    const { root, path, env } = fixture();
    const first = initializeConfig(env, path);
    expect(first.authSecret).toMatch(/^[0-9a-f]{64}$/);
    expect(first.encryptionKey?.bytes).toHaveLength(32);
    expect(initializeConfig(env, path)).toEqual(first);
    expect(readdirSync(root)).toEqual(["private"]);
    expect(readdirSync(join(root, "private"))).toEqual(["config.json"]);
    if (process.platform !== "win32") {
      expect(statSync(path).mode & 0o777).toBe(0o600);
      expect(statSync(join(root, "private")).mode & 0o777).toBe(0o700);
    }
  });

  it("persists supplied secrets on first launch and keeps later overrides ephemeral", () => {
    const { path, env } = fixture();
    expect(initializeConfig({ ...env, ...supplied }, path).authSecret).toBe(testAuthSecret);
    const contents = readFileSync(path, "utf8");
    const overridden = initializeConfig({ ...env, BETTER_AUTH_SECRET: "override-".repeat(8), AGENTS_ENCRYPTION_KEY: "34".repeat(32) }, path);
    expect(overridden.encryptionKey?.bytes).toEqual(new Uint8Array(Buffer.from("34".repeat(32), "hex")));
    expect(readFileSync(path, "utf8")).toBe(contents);
    expect(initializeConfig(env, path).authSecret).toBe(testAuthSecret);
  });

  it.each(["{broken-json", "{}", JSON.stringify({ version: 1, BETTER_AUTH_SECRET: testAuthSecret }), JSON.stringify({ version: 1, ...supplied, AGENTS_ENCRYPTION_KEY: "" }), JSON.stringify({ version: 2, ...supplied })])("does not replace malformed or incomplete configuration", (contents) => {
    const { root, path, env } = fixture();
    mkdirSync(join(root, "private"));
    writeFileSync(path, contents);
    expect(() => initializeConfig(env, path)).toThrow(/configuration/);
    expect(readFileSync(path, "utf8")).toBe(contents);
  });

  it("accepts complete external secrets without replacing damaged saved configuration", () => {
    const { root, path, env } = fixture();
    mkdirSync(join(root, "private"));
    writeFileSync(path, "{broken-json");
    const config = initializeConfig({ ...env, ...supplied, APP_PORT: "3010" }, path);
    expect(config.authSecret).toBe(testAuthSecret);
    expect(config.port).toBe(3010);
    expect(config.databasePath).toBe(env.APP_SQLITE_PATH);
    expect(readFileSync(path, "utf8")).toBe("{broken-json");
  });

  it.skipIf(process.platform === "win32")("protects the directory and file again on later launches", () => {
    const { root, path, env } = fixture();
    initializeConfig(env, path);
    chmodSync(join(root, "private"), 0o777);
    chmodSync(path, 0o666);
    initializeConfig(env, path);
    expect(statSync(join(root, "private")).mode & 0o777).toBe(0o700);
    expect(statSync(path).mode & 0o777).toBe(0o600);
  });

  it.each(["profile.sqlite", "profile.sqlite-wal", "profile.sqlite.lock"])("refuses a replacement key for existing installation marker %s", (marker) => {
    const { root, path, env } = fixture();
    writeFileSync(join(root, marker), "existing installation");
    expect(() => initializeConfig(env, path)).toThrow("original AGENTS_ENCRYPTION_KEY");
    expect(readdirSync(root)).toEqual([marker]);
  });

  it("accepts the original supplied key for an existing installation without touching its database", () => {
    const { path, env } = fixture();
    writeFileSync(env.APP_SQLITE_PATH, "unchanged");
    const config = initializeConfig({ ...env, ...supplied }, path);
    expect(config.encryptionKey?.bytes).toEqual(new Uint8Array(Buffer.from(supplied.AGENTS_ENCRYPTION_KEY, "hex")));
    expect(readFileSync(env.APP_SQLITE_PATH, "utf8")).toBe("unchanged");
    expect(initializeConfig(env, path)).toEqual(config);
  });

  it.each([{ BETTER_AUTH_SECRET: "short" }, { AGENTS_ENCRYPTION_KEY: "invalid" }, { AGENTS_ENCRYPTION_KEY: " " }, { APP_PORT: "invalid" }])("rejects invalid overrides without saving generated secrets", (override) => {
    const { root, path, env } = fixture();
    expect(() => initializeConfig({ ...env, ...override }, path)).toThrow();
    expect(readdirSync(root)).toEqual([]);
  });

  it("rejects an empty encryption override for a configured installation", () => {
    const { path, env } = fixture();
    initializeConfig(env, path);
    expect(() => initializeConfig({ ...env, AGENTS_ENCRYPTION_KEY: "" }, path)).toThrow("must not be empty");
  });

  it.skipIf(process.platform === "win32")("rejects a symbolic link instead of reading another file", () => {
    const { root, path, env } = fixture();
    const target = join(root, "target.json");
    writeFileSync(target, JSON.stringify({ version: 1, ...supplied }));
    mkdirSync(join(root, "private"));
    symlinkSync(target, path);
    expect(() => initializeConfig(env, path)).toThrow("regular file");
  });

  it("publishes one complete secret pair across concurrent processes", async () => {
    const { root, path, env } = fixture();
    const moduleUrl = new URL("./initialize-config.ts", import.meta.url).href;
    const script = `import { initializeConfig } from ${JSON.stringify(moduleUrl)}; initializeConfig(${JSON.stringify(env)}, ${JSON.stringify(path)});`;
    await Promise.all(Array.from({ length: 4 }, () => promisify(execFile)(process.execPath, ["--import", "tsx", "--input-type=module", "-e", script])));
    const contents = readFileSync(path, "utf8");
    const config = initializeConfig(env, path);
    expect(config.authSecret).toBe((JSON.parse(contents) as typeof supplied).BETTER_AUTH_SECRET);
    expect(readdirSync(join(root, "private"))).toEqual(["config.json"]);
  });
});

describe("private configuration paths", () => {
  it("uses platform directories and ignores relative base overrides", () => {
    expect(defaultConfigPath({}, "linux", "/home/test")).toBe("/home/test/.config/hypervibes/config.json");
    expect(defaultConfigPath({ XDG_CONFIG_HOME: "/private" }, "linux", "/home/test")).toBe("/private/hypervibes/config.json");
    expect(defaultConfigPath({ XDG_CONFIG_HOME: "relative" }, "linux", "/home/test")).toBe("/home/test/.config/hypervibes/config.json");
    expect(defaultConfigPath({}, "darwin", "/Users/test")).toBe("/Users/test/Library/Application Support/HyperVibes/config.json");
    expect(defaultConfigPath({}, "win32", "C:\\Users\\test")).toBe("C:\\Users\\test\\AppData\\Roaming\\HyperVibes\\config.json");
    expect(defaultConfigPath({ APPDATA: "D:\\Private" }, "win32", "C:\\Users\\test")).toBe("D:\\Private\\HyperVibes\\config.json");
    expect(defaultConfigPath({ APPDATA: "relative" }, "win32", "C:\\Users\\test")).toBe("C:\\Users\\test\\AppData\\Roaming\\HyperVibes\\config.json");
  });
});
