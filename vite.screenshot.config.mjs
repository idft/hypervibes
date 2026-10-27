import { isBuiltin } from "node:module";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const repository = fileURLToPath(new URL(".", import.meta.url));
const sourceRoot = resolve(repository, "src");
const fixtureRoot = resolve(sourceRoot, "screenshot");
const boundaryNames = new Set(["agents", "subagents", "indicators", "auth-client"]);
const browserWebModules = new Set([
  "app.css", "agent-queries.ts", "subagent-queries.ts", "subagent-mutations.ts", "indicator-queries.ts", "random-uuid.ts",
]);
const browserHyperliquidModules = new Set(["decimal.ts", "journal-decimal.ts", "currency-logo.ts"]);

function sourcePath(specifier, importer) {
  const clean = specifier.split(/[?#]/, 1)[0];
  if (isAbsolute(clean)) return resolve(clean);
  if (clean.startsWith(".") && importer) return resolve(dirname(importer.split("?", 1)[0]), clean);
  return null;
}

function canonicalSource(path) {
  // Browser source uses .js specifiers for TypeScript files.
  return path?.replace(/\.js$/, ".ts").replace(/\.jsx$/, ".tsx");
}

export function screenshotBoundariesPlugin() {
  function check(path) {
    const local = relative(sourceRoot, canonicalSource(path)).split(sep).join("/");
    if (local.startsWith("../") || local === "..") return;
    const [directory, filename] = local.split("/");
    const permitted = directory === "screenshot" || directory === "components"
      || (directory === "web" && browserWebModules.has(filename) && local === `web/${filename}`)
      || (directory === "hyperliquid" && browserHyperliquidModules.has(filename) && local === `hyperliquid/${filename}`);
    if (!permitted) throw new Error(`Screenshot build rejects backend-only source: ${local}`);
  }
  return {
    name: "screenshot-offline-boundaries",
    enforce: "pre",
    configResolved(config) {
      const output = resolve(config.root, config.build.outDir);
      const local = relative(resolve(tmpdir()), output).split(sep).join("/");
      if (!local.startsWith("hypervibes-screenshot-") || !local.includes("/") || local.split("/").includes("..")) {
        throw new Error("Screenshot build output must be inside an OS temporary hypervibes-screenshot-* directory");
      }
    },
    resolveId(specifier, importer) {
      if (specifier.startsWith("node:") || isBuiltin(specifier)) {
        throw new Error(`Screenshot build rejects Node module: ${specifier}`);
      }
      if (/^(?:@earendil-works\/|@nktkas\/)/.test(specifier) || /^(?:@tanstack\/react-start|better-auth|drizzle-orm|fastify|viem)(?:\/|$)/.test(specifier)) {
        throw new Error(`Screenshot build rejects server dependency: ${specifier}`);
      }
      const path = sourcePath(specifier, importer);
      if (!path) return null;
      const canonical = canonicalSource(path);
      if (dirname(canonical) === resolve(sourceRoot, "web") && boundaryNames.has(canonical.split(sep).at(-1).replace(/\.ts$/, ""))) {
        return resolve(fixtureRoot, "screenshot-boundaries.ts");
      }
      if (canonical === resolve(sourceRoot, "components/agent-events.ts") || canonical === resolve(sourceRoot, "components/agent-events.tsx")) {
        return resolve(fixtureRoot, "screenshot-events.tsx");
      }
      check(path);
      return null;
    },
    load(id) {
      if (isAbsolute(id.split("?", 1)[0])) check(id.split("?", 1)[0]);
      return null;
    },
  };
}

export default defineConfig({
  root: fixtureRoot,
  publicDir: false,
  envDir: false,
  envPrefix: "SCREENSHOT_PUBLIC_",
  plugins: [screenshotBoundariesPlugin(), react(), tailwindcss()],
  build: {
    emptyOutDir: true,
    sourcemap: false,
    assetsInlineLimit: 0,
    // The runner must supply an OS temporary output directory.
    write: true,
  },
});
