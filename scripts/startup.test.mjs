import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { assertSupportedNode, minimumNodeVersion } from "./check-node.mjs";

const packageJson = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8"));
const checker = await readFile(new URL("./check-node.mjs", import.meta.url), "utf8");
const cli = await readFile(new URL("../bin/hypervibes.js", import.meta.url), "utf8");

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

test("package scripts build compiled releases and expose a build-free executable", () => {
  assert.equal(packageJson.engines.node, `>=${minimumNodeVersion}`);
  assert.equal(packageJson.scripts.start, "pnpm build && node bin/hypervibes.js");
  assert.equal(packageJson.scripts.build, "node scripts/check-node.mjs && pnpm build:app && pnpm build:server");
  assert.equal(packageJson.scripts["start:built"], "node bin/hypervibes.js");
  assert.equal(packageJson.scripts["start:server"], undefined);
  assert.equal(packageJson.scripts.dev, "node --env-file-if-exists=.env --import tsx src/main.ts --dev");
  assert.equal(packageJson.scripts["dev:server"], undefined);
  assert.equal(packageJson.bin.hypervibes, "./bin/hypervibes.js");
  assert.equal(packageJson.dependencies.tsx, undefined);
  assert.ok(packageJson.devDependencies.tsx);
});

test("startup rebuilds and stops after a failed build or Node.js check", async () => {
  const directory = await mkdtemp(join(tmpdir(), "hypervibes-startup-"));
  try {
    await mkdir(join(directory, "scripts"));
    await mkdir(join(directory, "bin"));
    await writeFile(join(directory, "scripts/check-node.mjs"), checker);
    await writeFile(join(directory, "bin/hypervibes.js"), 'import "../fixture.mjs";');
    await writeFile(join(directory, "package.json"), JSON.stringify({
      private: true,
      type: "module",
      scripts: {
        start: packageJson.scripts.start,
        build: packageJson.scripts.build,
        "build:app": "node fixture.mjs build",
        "build:server": "node fixture.mjs compile",
      },
    }));
    await writeFile(join(directory, "fixture.mjs"), `
      import { appendFileSync } from "node:fs";
      appendFileSync("events.txt", (process.argv[2] ?? "server") + "\\n");
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
    const successful = "build\ncompile\nserver\nbuild\ncompile\nserver\n";
    assert.equal(await readFile(join(directory, "events.txt"), "utf8"), successful);
    const failedBuild = run({ FAIL_BUILD: "1" });
    assert.notEqual(failedBuild.status, 0);
    assert.equal(await readFile(join(directory, "events.txt"), "utf8"), successful + "build\n");

    await writeFile(join(directory, "old-node.mjs"), 'Object.defineProperty(process.versions, "node", { value: "22.23.2" });');
    const failedCheck = run({ NODE_OPTIONS: "--import ./old-node.mjs" });
    assert.notEqual(failedCheck.status, 0);
    assert.match(failedCheck.stderr, /requires Node\.js >=22\.23\.3; found 22\.23\.2/);
    assert.equal(await readFile(join(directory, "events.txt"), "utf8"), successful + "build\n");
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("CLI runs compiled code from an unrelated directory and loads optional env without overriding exports", async () => {
  const directory = await mkdtemp(join(tmpdir(), "hypervibes-cli-"));
  try {
    for (const folder of ["bin", "scripts", "dist", "working"]) await mkdir(join(directory, folder));
    await writeFile(join(directory, "bin/hypervibes.js"), cli);
    await writeFile(join(directory, "scripts/check-node.mjs"), checker);
    await writeFile(join(directory, "package.json"), JSON.stringify({ type: "module", version: "1.2.3" }));
    await writeFile(join(directory, "dist/main.js"), `
      export async function main(args) {
        console.log(JSON.stringify({ args, setting: process.env.HYPERVIBES_CLI_TEST ?? null }));
      }
    `);
    const run = (args = [], environment = {}) => spawnSync(process.execPath, [join(directory, "bin/hypervibes.js"), ...args], {
      cwd: join(directory, "working"),
      env: { ...process.env, HYPERVIBES_CLI_TEST: "", ...environment },
      encoding: "utf8",
      timeout: 10_000,
    });
    const first = run();
    assert.equal(first.status, 0, first.stderr);
    assert.deepEqual(JSON.parse(first.stdout), { args: [], setting: "" });
    // An empty env value is still an explicit override. Remove it for the file test.
    await writeFile(join(directory, "working/.env"), "HYPERVIBES_CLI_TEST=from-file\n");
    const fromFile = run([], { HYPERVIBES_CLI_TEST: undefined });
    assert.equal(fromFile.status, 0, fromFile.stderr);
    assert.equal(JSON.parse(fromFile.stdout).setting, "from-file");
    const overridden = run([], { HYPERVIBES_CLI_TEST: "exported" });
    assert.equal(JSON.parse(overridden.stdout).setting, "exported");
    assert.match(run(["--help"]).stdout, /Usage: hypervibes/);
    assert.equal(run(["--version"]).stdout.trim(), "1.2.3");
    assert.notEqual(run(["--unknown"]).status, 0);
    assert.notEqual(run(["--dev"]).status, 0);

    // Informational commands must not import the backend or initialize user files.
    await rm(join(directory, "dist"), { recursive: true });
    assert.equal(run(["--help"]).status, 0);
    assert.equal(run(["--version"]).status, 0);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
