#!/usr/bin/env node
import { readFile } from "node:fs/promises";
import { assertSupportedNode } from "../scripts/check-node.mjs";

try {
  assertSupportedNode();
  const args = process.argv.slice(2);
  if (args.length === 1 && ["--help", "-h"].includes(args[0])) {
    console.log("Usage: hypervibes [--help | --version]\n\nStarts HyperVibes at http://localhost:3003 by default.\nConfigure with environment variables or an optional .env in the current directory.\nPress Ctrl+C to stop.");
  } else if (args.length === 1 && ["--version", "-v"].includes(args[0])) {
    const { version } = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8"));
    console.log(version);
  } else if (args.length !== 0) {
    throw new Error("Unknown arguments. Run hypervibes --help for usage.");
  } else {
    try { process.loadEnvFile(); }
    catch (error) { if (error.code !== "ENOENT") throw error; }
    const { main } = await import("../dist/main.js");
    await main([]);
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
}
