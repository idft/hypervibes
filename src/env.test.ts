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

function serverCommand(script: string) {
  const command = script.split("&&")
    .map((part) => part.trim().split(/\s+/))
    .find((part) => part.includes("src/main.ts"));
  expect(command, "script must contain the server entry command").toBeDefined();
  return command!;
}

describe("server .env loading", () => {
  let directory: string;
  beforeEach(() => { directory = mkdtempSync(join(tmpdir(), "hypervibes-env-")); });
  afterEach(() => rmSync(directory, { recursive: true, force: true }));

  function load(script: string, env: NodeJS.ProcessEnv = {}) {
    const command = serverCommand(scripts[script]!);
    expect(command[0]).toBe("node");
    expect(command).toContain("--env-file-if-exists=.env");
    const entryIndex = command.indexOf("src/main.ts");
    expect(entryIndex).toBeGreaterThan(0);
    const args = command.slice(1, entryIndex);
    args[args.indexOf("--import") + 1] = fileURLToPath(import.meta.resolve("tsx"));
    // Exercise the real CLI flags, but import only config: no listener or database.
    const output = execFileSync(process.execPath, [...args, "--input-type=module", "-e", `
      import { loadConfig } from ${JSON.stringify(configUrl)};
      const config = loadConfig();
      console.log(JSON.stringify({ host: config.host, port: config.port, origin: config.authOrigin, secret: config.authSecret }));
    `], { cwd: directory, env: { PATH: process.env.PATH, ...env }, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    return JSON.parse(output) as { host: string; port: number; origin: string; secret: string };
  }

  it("extracts the server command after a startup guard", () => {
    expect(serverCommand(`node scripts/check-node.mjs && ${scripts["start:server"]!}`))
      .toEqual(serverCommand(scripts["start:server"]!));
  });

  it.each(["dev:server", "start:server"])("loads local settings with %s", (script) => {
    writeFileSync(join(directory, ".env"), `APP_PORT=3103\nAPP_ORIGIN=http://localhost:3103\nBETTER_AUTH_SECRET=${testAuthSecret}\n`);
    expect(load(script)).toEqual({ host: "127.0.0.1", port: 3103, origin: "http://localhost:3103", secret: testAuthSecret });
  });

  it("loads explicit private LAN HTTP settings with start:server", () => {
    writeFileSync(join(directory, ".env"), `APP_HOST=0.0.0.0\nAPP_ORIGIN=http://10.16.20.111:3003\nAPP_ALLOW_INSECURE_HTTP=true\nBETTER_AUTH_SECRET=${testAuthSecret}\n`);
    expect(load("start:server")).toEqual({ host: "0.0.0.0", port: 3003, origin: "http://10.16.20.111:3003", secret: testAuthSecret });
  });

  it("lets shell variables override file settings", () => {
    writeFileSync(join(directory, ".env"), `APP_PORT=3103\nBETTER_AUTH_SECRET=${testAuthSecret}\n`);
    expect(load("dev:server", { APP_PORT: "3203" }).port).toBe(3203);
  });

  it("allows a missing .env when the required secret is supplied externally", () => {
    expect(load("start:server", { BETTER_AUTH_SECRET: testAuthSecret })).toEqual({
      host: "127.0.0.1", port: 3003, origin: "http://localhost:3003", secret: testAuthSecret,
    });
  });
});
