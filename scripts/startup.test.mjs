import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { assertSupportedNode, minimumNodeVersion } from "./check-node.mjs";

const packageJson = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8"));
const checker = await readFile(new URL("./check-node.mjs", import.meta.url), "utf8");

for (const version of ["22.23.3", "22.23.4", "22.24.0", "23.0.0", "24.0.0"]) {
  test(`accepts Node.js ${version}`, () => assert.doesNotThrow(() => assertSupportedNode(version)));
}

for (const version of ["18.20.8", "20.99.0", "22.22.99", "22.23.2", "22.23.3-rc.1", "invalid"]) {
  test(`rejects Node.js ${version}`, () => {
    assert.throws(() => assertSupportedNode(version), {
      message: `HyperVibes requires Node.js >=${minimumNodeVersion}; found ${version}. Upgrade Node.js before building or starting HyperVibes.`,
    });
  });
}

test("package scripts check Node.js before build and preserve direct server startup", () => {
  assert.equal(packageJson.engines.node, `>=${minimumNodeVersion}`);
  assert.equal(packageJson.scripts.start, "node scripts/check-node.mjs && pnpm build:app && pnpm start:server");
  assert.equal(packageJson.scripts["start:server"], "node --env-file-if-exists=.env --import tsx src/main.ts");
  assert.ok(packageJson.dependencies.tsx);
  assert.equal(packageJson.devDependencies.tsx, undefined);
});

test("startup builds on each invocation and stops after a failed build or Node.js check", async () => {
  const directory = await mkdtemp(join(tmpdir(), "hypervibes-startup-"));
  try {
    await mkdir(join(directory, "scripts"));
    await writeFile(join(directory, "scripts/check-node.mjs"), checker);
    await writeFile(join(directory, "package.json"), JSON.stringify({
      private: true,
      scripts: {
        start: packageJson.scripts.start,
        "build:app": "node fixture.mjs build",
        "start:server": "node fixture.mjs server",
      },
    }));
    await writeFile(join(directory, "fixture.mjs"), `
      import { appendFileSync } from "node:fs";
      appendFileSync("events.txt", process.argv[2] + "\\n");
      if (process.env.FAIL_BUILD === "1" && process.argv[2] === "build") process.exitCode = 7;
    `);
    const run = (environment = {}) => spawnSync(process.execPath, ["--run", "start"], {
      cwd: directory,
      env: { ...process.env, ...environment },
      encoding: "utf8",
      timeout: 30_000,
    });
    for (let invocation = 0; invocation < 2; invocation++) {
      const result = run();
      assert.equal(result.status, 0, result.stdout + result.stderr);
    }
    assert.equal(await readFile(join(directory, "events.txt"), "utf8"), "build\nserver\nbuild\nserver\n");
    const failedBuild = run({ FAIL_BUILD: "1" });
    assert.notEqual(failedBuild.status, 0);
    assert.equal(await readFile(join(directory, "events.txt"), "utf8"), "build\nserver\nbuild\nserver\nbuild\n");

    // Use the real check with an old runtime version, without installing an old Node.js.
    await writeFile(join(directory, "old-node.mjs"), 'Object.defineProperty(process.versions, "node", { value: "22.23.2" });');
    const failedCheck = run({ NODE_OPTIONS: "--import ./old-node.mjs" });
    assert.notEqual(failedCheck.status, 0);
    assert.match(failedCheck.stderr, /requires Node\.js >=22\.23\.3; found 22\.23\.2/);
    assert.equal(await readFile(join(directory, "events.txt"), "utf8"), "build\nserver\nbuild\nserver\nbuild\n");
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
