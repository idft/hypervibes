import { spawnSync } from "node:child_process";
import { cp, mkdir, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { assertSupportedNode } from "./check-node.mjs";

assertSupportedNode();
const root = new URL("../", import.meta.url);
const output = new URL("dist/", root);
// Only this generated directory is removed. User data is stored outside it.
await rm(output, { recursive: true, force: true });
const result = spawnSync(process.execPath, [
  fileURLToPath(new URL("node_modules/typescript/bin/tsc", root)),
  "--project", fileURLToPath(new URL("tsconfig.build.json", root)),
], { cwd: fileURLToPath(root), stdio: "inherit" });
if (result.error) throw result.error;
if (result.status !== 0) {
  process.exitCode = result.status ?? 1;
} else {
  for (const directory of ["db/migrations", "indicators/builtins"]) {
    const destination = new URL(`${directory}/`, output);
    await mkdir(destination, { recursive: true });
    await cp(new URL(`src/${directory}/`, root), destination, {
      recursive: true,
      filter: (source) => !source.endsWith(".json"), // Migration snapshots are development-only.
    });
  }
}
