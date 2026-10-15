import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppConfig } from "../config.js";
import type { AppRuntime } from "../runtime.js";
import { createApp } from "./app.js";
import type { StartApplication } from "./start.js";

describe("embedded HTTP host", () => {
  let directory: string;
  let config: AppConfig;
  let app: Awaited<ReturnType<typeof createApp>>;
  let start: StartApplication;
  let runtimes: AppRuntime[];
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hypervibes-http-"));
    config = {
      host: "127.0.0.1",
      port: 3003,
      databasePath: join(directory, "data", "test.sqlite"),
      staticDir: join(directory, "static"),
      buildDir: join(directory, "build"),
    };
    mkdirSync(join(config.staticDir, "dist"), { recursive: true });
    mkdirSync(join(config.buildDir, "client", "assets"), { recursive: true });
    writeFileSync(join(config.staticDir, "dist", "app.js"), "legacy fixture");
    writeFileSync(join(config.buildDir, "client", "assets", "app-123.js"), "Start fixture");
    writeFileSync(join(config.staticDir, "icon.svg"), "<svg></svg>");
    writeFileSync(join(config.staticDir, ".env"), "hidden-secret");
    writeFileSync(join(directory, "secret.txt"), "outside-secret");
    runtimes = [];
    start = {
      close: vi.fn(async () => {}),
      async fetch(request, { context }) {
        runtimes.push(context.runtime);
        if (new URL(request.url).pathname === "/echo") {
          return new Response(await request.text(), { headers: { "Content-Type": request.headers.get("Content-Type") ?? "text/plain" } });
        }
        if (new URL(request.url).pathname === "/") {
          const headers = new Headers({ "Content-Type": "text/html" });
          headers.append("Set-Cookie", "first=1; HttpOnly; Path=/");
          headers.append("Set-Cookie", "second=2; HttpOnly; Path=/");
          return new Response(`<h1>Start: SQLite ${context.runtime.status().database}</h1>`, { headers });
        }
        return new Response("Not found", { status: 404 });
      },
    };
    // Inject only the web handler, never Vite or a listening server.
    app = await createApp(config, { loadWeb: async () => start });
  });
  afterEach(async () => {
    await app.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("injects one application runtime across Start requests without listening", async () => {
    expect(app.server.listening).toBe(false);
    expect((await app.inject("/healthz")).json()).toEqual({ status: "ok" });
    for (let count = 0; count < 2; count++) {
      const root = await app.inject("/");
      expect(root.statusCode).toBe(200);
      expect(root.headers["content-type"]).toContain("text/html");
      expect(root.body).toContain("SQLite connected");
      expect(root.headers["set-cookie"]).toEqual(["first=1; HttpOnly; Path=/", "second=2; HttpOnly; Path=/"]);
    }
    expect(runtimes).toHaveLength(2);
    expect(runtimes[0]).toBe(runtimes[1]);
    expect((await app.inject("/api/v1/agents")).statusCode).toBe(404);
  });

  it.each(["application/json", "application/x-www-form-urlencoded", "multipart/form-data; boundary=fixture"])("preserves the request body for Start: %s", async (contentType) => {
    const response = await app.inject({ method: "POST", url: "/echo", headers: { "Content-Type": contentType }, payload: "body fixture" });
    expect(response.statusCode).toBe(200);
    expect(response.body).toBe("body fixture");
  });

  it("serves legacy and Start assets without delegating to SSR", async () => {
    const asset = await app.inject("/static/dist/app.js");
    expect(asset.statusCode).toBe(200);
    expect(asset.body).toBe("legacy fixture");
    expect(asset.headers["cache-control"]).toBe("no-store");
    const bundle = await app.inject("/assets/app-123.js");
    expect(bundle.statusCode).toBe(200);
    expect(bundle.body).toBe("Start fixture");
    expect((await app.inject("/static/icon.svg")).statusCode).toBe(200);
    const head = await app.inject({ method: "HEAD", url: "/static/dist/app.js" });
    expect(head.statusCode).toBe(200);
    expect(head.body).toBe("");
    expect(runtimes).toHaveLength(0);
  });

  it.each(["/static/missing.js", "/static/", "/static/.env", "/static/%2e%2e/secret.txt", "/assets/%2e%2e/secret.txt", "/src/main.ts"])("does not expose non-public files: %s", async (url) => {
    const response = await app.inject(url);
    expect(response.statusCode).toBeGreaterThanOrEqual(400);
    expect(response.body).not.toContain("outside-secret");
    expect(response.body).not.toContain("hidden-secret");
  });

  it("closes the web application before its runtime and permits reopening", async () => {
    await app.inject("/");
    const runtime = runtimes[0]!;
    start.close = vi.fn(async () => { expect(runtime.status().database).toBe("connected"); });
    await app.close();
    expect(start.close).toHaveBeenCalledOnce();
    expect(runtime.db.$client.isOpen).toBe(false);
    app = await createApp(config, { loadWeb: async () => ({ ...start, close: async () => {} }) });
    expect((await app.inject("/healthz")).statusCode).toBe(200);
  });

  it.each(["static", "build/client"])("rejects SQLite beneath a public directory: %s", async (path) => {
    await expect(createApp({ ...config, databasePath: join(directory, path, "db.sqlite") }))
      .rejects.toThrow("outside public");
  });

  it("reports a missing production build without leaving an initialized app", async () => {
    await expect(createApp(config)).rejects.toThrow("pnpm build:app");
  });

  it("can import the entry point without starting a server", async () => {
    expect((await import("../main.js")).main).toBeTypeOf("function");
    expect(app.server.listening).toBe(false);
  });
});
