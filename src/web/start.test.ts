import { resolve } from "node:path";
import { isFileLoadingAllowed, isFileServingAllowed, normalizePath, resolveConfig } from "vite";
import { expect, it } from "vitest";
import { developmentFileAccess } from "./start.js";

it("denies screenshot source and /@fs URLs in normal development without starting a server", async () => {
  const config = await resolveConfig({ configFile: false, envDir: false,
    server: { fs: developmentFileAccess(resolve("unused-test-database.sqlite")) } }, "serve");
  for (const path of ["index.html", "screenshot-entry.tsx", "dashboard-fixture.ts", "currency/BTC.svg"]) {
    // Normal /src URLs resolve against Vite's root before its file-loading access check.
    const source = normalizePath(resolve(config.root, "src/screenshot", path));
    expect(isFileLoadingAllowed(config, source)).toBe(false);
    expect(isFileServingAllowed(config, `/@fs/${source}`)).toBe(false);
  }
  expect(isFileLoadingAllowed(config, normalizePath(resolve("src/components/agent-dashboard-page.tsx")))).toBe(true);
  expect(isFileLoadingAllowed(config, normalizePath(resolve(".env")))).toBe(false);
  expect(isFileLoadingAllowed(config, normalizePath(resolve("unused-test-database.sqlite")))).toBe(false);
});
