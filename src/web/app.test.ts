import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { privateKeyToAccount } from "viem/accounts";
import { createSiweMessage } from "viem/siwe";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppConfig } from "../config.js";
import type { AppRuntime } from "../runtime.js";
import { testAuthSecret } from "../test-config.js";
import { AgentEventStreams } from "./agent-events.js";
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
      authOrigin: "http://localhost",
      authSecret: testAuthSecret,
      databasePath: join(directory, "data", "test.sqlite"),
      staticDir: join(directory, "static"),
      buildDir: join(directory, "build"),
      encryptionKey: undefined,
      cacheDir: join(directory, "cache"),
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
    app = await createApp(config, { loadWeb: async () => start, logoFetcher: async () => new Response('<svg xmlns="http://www.w3.org/2000/svg"></svg>', { headers: { "Content-Type": "image/svg+xml" } }) });
  });
  afterEach(async () => {
    await app.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("serves cached currency SVGs with restrictive headers outside the Start catch-all", async () => {
    const response = await app.inject("/currency/BTC.svg");
    expect(response.statusCode).toBe(200);
    expect(response.headers["content-type"]).toContain("image/svg+xml");
    expect(response.headers["cache-control"]).toBe("public, max-age=86400");
    expect(response.headers["x-content-type-options"]).toBe("nosniff");
    expect(response.headers["content-security-policy"]).toContain("sandbox; default-src 'none'");
    expect(response.body).toContain("<svg");
    expect((await app.inject("/currency/BTC.png")).statusCode).toBe(400);
    expect((await app.inject("/currency/%40107.svg")).statusCode).toBe(400);
    expect((await app.inject("/currency/a%2Fb.svg")).statusCode).toBe(400);
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

  it("serves the Agent chat event stream through the embedded host", async () => {
    // The stream route is registered before the Start catch-all; anonymous
    // browsers receive a finite JSON error, never an open stream.
    const denied = await app.inject("/api/agents/4527ece2-5a90-4d5e-b73a-351368ad71f3/events");
    expect(denied.statusCode).toBe(401);
    expect(denied.headers["content-type"]).toContain("application/json");
    expect(denied.headers["cache-control"]).toBe("no-store");
    expect(denied.json<{ error: string }>().error).toContain("Sign in");
    const crossSite = await app.inject({ url: "/api/agents/4527ece2-5a90-4d5e-b73a-351368ad71f3/events", headers: { origin: "https://evil.example" } });
    expect(crossSite.statusCode).toBe(403);
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

  it("serves real Better Auth nonce, verification, session and logout endpoints without delegating to SSR", async () => {
    const wallet = privateKeyToAccount(`0x${"33".repeat(32)}`);
    const nonceResponse = await app.inject({ method: "POST", url: "/api/auth/siwe/nonce", headers: { origin: config.authOrigin }, payload: {} });
    expect(nonceResponse.statusCode).toBe(200);
    expect(nonceResponse.headers["cache-control"]).toBe("no-store");
    const binding = nonceResponse.cookies.map((cookie) => `${cookie.name}=${cookie.value}`).join("; ");
    const now = new Date();
    const message = createSiweMessage({ address: wallet.address, domain: "localhost", uri: `${config.authOrigin}/login`, chainId: 1, version: "1", nonce: nonceResponse.json<{ nonce: string }>().nonce, issuedAt: now, expirationTime: new Date(now.getTime() + 300_000) });
    const verify = await app.inject({ method: "POST", url: "/api/auth/siwe/verify", headers: { origin: config.authOrigin, cookie: binding, "x-hv-client-ip": "203.0.113.99", "x-forwarded-for": "203.0.113.98" }, payload: { message, signature: await wallet.signMessage({ message }) } });
    expect(verify.statusCode).toBe(200);
    const cookie = verify.cookies.filter((item) => item.name.includes("session_token")).map((item) => `${item.name}=${item.value}`).join("; ");
    const session = await app.inject({ url: "/api/auth/get-session", headers: { cookie } });
    expect(session.json<{ user: { name: string }; session: { ipAddress: string } }>().user.name).toBe(wallet.address);
    expect(session.json<{ session: { ipAddress: string } }>().session.ipAddress).toBe("127.0.0.1");
    expect((await app.inject({ method: "POST", url: "/api/auth/sign-out", headers: { origin: "https://evil.example", cookie }, payload: {} })).statusCode).toBe(403);
    expect((await app.inject({ method: "POST", url: "/api/auth/sign-out", headers: { origin: config.authOrigin, cookie }, payload: {} })).statusCode).toBe(200);
    expect((await app.inject({ url: "/api/auth/get-session", headers: { cookie } })).json()).toBeNull();
    expect(runtimes).toHaveLength(0);
  });

  it("rejects an untrusted host and ignores forged proxy origin headers", async () => {
    expect((await app.inject({ url: "/", headers: { host: "evil.example" } })).statusCode).toBe(400);
    const response = await app.inject({ method: "POST", url: "/api/auth/siwe/nonce", headers: { origin: "https://evil.example", "x-forwarded-host": "evil.example", "x-forwarded-proto": "https" }, payload: {} });
    expect(response.statusCode).toBe(403);
    expect(runtimes).toHaveLength(0);
  });

  it("closes chat and dashboard streams before HTTP draining and releases the profile lock", async () => {
    await app.inject("/");
    const runtime = runtimes[0]!;
    const closeStreams = vi.spyOn(AgentEventStreams.prototype, "close");
    const closeServer = app.server.close.bind(app.server);
    const drain = vi.spyOn(app.server, "close").mockImplementation((callback) => {
      expect(closeStreams).toHaveBeenCalledTimes(2);
      expect(runtime.status().database).toBe("connected");
      expect(start.close).not.toHaveBeenCalled();
      return closeServer(callback);
    });
    try {
      await app.close();
      expect(drain).toHaveBeenCalledOnce();
      expect(start.close).toHaveBeenCalledOnce();
      await expect(runtime.database.get("SELECT 1")).rejects.toThrow("closed");
      app = await createApp(config, { loadWeb: async () => start });
      expect((await app.inject("/healthz")).statusCode).toBe(200);
    } finally {
      drain.mockRestore();
      closeStreams.mockRestore();
    }
  });

  it("closes the web application before its runtime and permits reopening", async () => {
    await app.inject("/");
    const runtime = runtimes[0]!;
    start.close = vi.fn(async () => { expect(runtime.status().database).toBe("connected"); });
    await app.close();
    expect(start.close).toHaveBeenCalledOnce();
    await expect(runtime.database.get("SELECT 1")).rejects.toThrow("closed");
    app = await createApp(config, { loadWeb: async () => ({ ...start, close: async () => {} }) });
    expect((await app.inject("/healthz")).statusCode).toBe(200);
  });

  it.each(["static", "build/client"])("rejects SQLite beneath a public directory: %s", async (path) => {
    await expect(createApp({ ...config, databasePath: join(directory, path, "db.sqlite") }))
      .rejects.toThrow("outside public");
  });

  it("reports a missing production build without leaving an initialized app", async () => {
    await app.close();
    await expect(createApp(config)).rejects.toThrow("pnpm build:app");
    app = await createApp(config, { loadWeb: async () => start });
  });

  it("can import the entry point without starting a server", async () => {
    expect((await import("../main.js")).main).toBeTypeOf("function");
    expect(app.server.listening).toBe(false);
  });
});
