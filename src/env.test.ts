import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { testAuthSecret } from "./test-config.js";

const { scripts } = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")) as {
  scripts: Record<string, string>;
};
const configUrl = new URL("./config.ts", import.meta.url).href;

describe("development .env loading", () => {
  let directory: string;
  beforeEach(() => { directory = mkdtempSync(join(tmpdir(), "hypervibes-env-")); });
  afterEach(() => rmSync(directory, { recursive: true, force: true }));

  function load(env: NodeJS.ProcessEnv = {}) {
    const command = scripts.dev!.split(/\s+/);
    expect(command[0]).toBe("node");
    expect(command).toContain("--env-file-if-exists=.env");
    const entryIndex = command.indexOf("src/main.ts");
    expect(entryIndex).toBeGreaterThan(0);
    const args = command.slice(1, entryIndex);
    args[args.indexOf("--import") + 1] = fileURLToPath(import.meta.resolve("tsx"));
    // Exercise the development flags without opening a listener or database.
    // The compiled CLI's env loading is covered by scripts/startup.test.mjs.
    const output = execFileSync(process.execPath, [...args, "--input-type=module", "-e", `
      import { loadConfig } from ${JSON.stringify(configUrl)};
      const config = loadConfig();
      console.log(JSON.stringify({ host: config.host, port: config.port, origin: config.authOrigin, secret: config.authSecret }));
    `], { cwd: directory, env: { PATH: process.env.PATH, ...env }, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    return JSON.parse(output) as { host: string; port: number; origin: string; secret: string };
  }

  it("loads local settings with pnpm dev", () => {
    writeFileSync(join(directory, ".env"), `APP_PORT=3103\nAPP_ORIGIN=http://localhost:3103\nBETTER_AUTH_SECRET=${testAuthSecret}\n`);
    expect(load()).toEqual({ host: "127.0.0.1", port: 3103, origin: "http://localhost:3103", secret: testAuthSecret });
  });

  it("loads explicit private LAN HTTP settings", () => {
    writeFileSync(join(directory, ".env"), `APP_HOST=0.0.0.0\nAPP_ORIGIN=http://10.16.20.111:3003\nAPP_ALLOW_INSECURE_HTTP=true\nBETTER_AUTH_SECRET=${testAuthSecret}\n`);
    expect(load()).toEqual({ host: "0.0.0.0", port: 3003, origin: "http://10.16.20.111:3003", secret: testAuthSecret });
  });

  it("lets shell variables override file settings", () => {
    writeFileSync(join(directory, ".env"), `APP_PORT=3103\nBETTER_AUTH_SECRET=${testAuthSecret}\n`);
    expect(load({ APP_PORT: "3203" }).port).toBe(3203);
  });

  it("allows a missing .env when the required secret is supplied externally", () => {
    expect(load({ BETTER_AUTH_SECRET: testAuthSecret })).toEqual({
      host: "127.0.0.1", port: 3003, origin: "http://localhost:3003", secret: testAuthSecret,
    });
  });
});
