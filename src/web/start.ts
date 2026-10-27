import { access } from "node:fs/promises";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import type { Connect, FileSystemServeOptions } from "vite";
import type { AppConfig } from "../config.js";
import type { AppRuntime } from "../runtime.js";

export interface StartEntry {
  fetch(request: Request, options: { context: { runtime: AppRuntime } }): Response | Promise<Response>;
}

export interface StartApplication extends StartEntry {
  middleware?: Connect.Server;
  close(): Promise<void>;
}

/** Screenshot sources belong only to the temporary screenshot build, not normal development. */
export function developmentFileAccess(databasePath: string): FileSystemServeOptions {
  return {
    allow: [resolve("src"), resolve("assets"), resolve("node_modules")],
    deny: [".env", ".env.*", "*.{crt,pem}", "**/.git/**", "**/*.sqlite*", "**/*.db*", databasePath,
      `${resolve("src/screenshot").replaceAll("\\", "/")}/**`],
  };
}

/** Loads the web handler only; the application host owns the runtime and listener. */
export async function loadStart(config: AppConfig, development: boolean): Promise<StartApplication> {
  if (development) {
    const { createServer, isRunnableDevEnvironment } = await import("vite");
    const vite = await createServer({
      configFile: resolve("vite.app.config.ts"),
      appType: "custom",
      server: {
        middlewareMode: true,
        hmr: false, // No extra listener or runtime restart; refresh the browser after edits.
        fs: developmentFileAccess(config.databasePath),
      },
    });
    try {
      const environment = vite.environments.ssr;
      if (!environment || !isRunnableDevEnvironment(environment)) {
        throw new Error("Start requires a runnable Vite SSR environment");
      }
      return {
        middleware: vite.middlewares,
        async fetch(request, options) {
          const entry = await environment.runner.import<{ default: StartEntry }>("virtual:tanstack-start-server-entry");
          return entry.default.fetch(request, options);
        },
        close: () => vite.close(),
      };
    } catch (error) {
      await vite.close();
      throw error;
    }
  }

  const entryPath = join(config.buildDir, "server", "server.js");
  try {
    await access(entryPath);
  } catch (cause) {
    throw new Error("Start build not found. Run pnpm build:app first, or use pnpm dev.", { cause });
  }
  const entry: { default: StartEntry } = await import(pathToFileURL(entryPath).href);
  return {
    fetch: (request, options) => entry.default.fetch(request, options),
    close: async () => {},
  };
}
