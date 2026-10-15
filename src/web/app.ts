import fastifyStatic from "@fastify/static";
import Fastify from "fastify";
import { isAbsolute, join, relative, sep } from "node:path";
import { NodeRequest, sendNodeResponse } from "srvx/node";
import type { AppConfig } from "../config.js";
import { createRuntime } from "../runtime.js";
import { runMiddleware } from "./middleware.js";
import { loadStart } from "./start.js";
import type { StartApplication } from "./start.js";

interface AppOptions {
  logger?: boolean;
  development?: boolean;
  loadWeb?: typeof loadStart;
}

/** Constructs a single embedded runtime and web host, without opening a listener. */
export async function createApp(config: AppConfig, options: AppOptions = {}) {
  for (const publicDir of [config.staticDir, join(config.buildDir, "client")]) {
    const location = relative(publicDir, config.databasePath);
    if (!isAbsolute(location) && location !== ".." && !location.startsWith(`..${sep}`)) {
      throw new Error("APP_SQLITE_PATH must be outside public static and client directories");
    }
  }

  const app = Fastify({
    logger: options.logger ? { redact: ["req.headers.cookie", "req.headers.authorization", "res.headers.set-cookie"] } : false,
  });
  const origin = new URL(config.authOrigin);
  app.addHook("onRequest", async (request, reply) => {
    let host: string | undefined;
    try { host = new URL(`${origin.protocol}//${request.headers.host ?? ""}`).host; } catch { /* Reject malformed hosts below. */ }
    if (host !== origin.host) return reply.code(400).send({ error: "Invalid request host" });
  });
  const runtime = await createRuntime(config);
  let web: StartApplication | undefined;
  app.addHook("onClose", async () => {
    try { await web?.close(); }
    finally { runtime.close(); }
  });

  try {
    const development = options.development ?? false;
    web = await (options.loadWeb ?? loadStart)(config, development);
    const start = web;

    app.register(fastifyStatic, {
      root: config.staticDir,
      prefix: "/static/",
      dotfiles: "deny",
      index: false,
      cacheControl: false,
      setHeaders(response, path) {
        if (relative(config.staticDir, path).startsWith(`dist${sep}`)) {
          response.header("Cache-Control", "no-store");
        }
      },
    });
    if (!development) {
      app.register(fastifyStatic, {
        root: join(config.buildDir, "client", "assets"),
        prefix: "/assets/",
        decorateReply: false,
        dotfiles: "deny",
        index: false,
      });
    }
    app.get("/healthz", async () => {
      runtime.status();
      return { status: "ok" };
    });

    // Leave request streams unread for Start (JSON, FormData, etc.). The Node
    // adapter enforces a body limit when Start reads buffered or streamed input.
    app.removeAllContentTypeParsers();
    app.addContentTypeParser("*", (_request, payload, done) => done(null, payload));
    app.all("/*", async (request, reply) => {
      reply.hijack();
      try {
        if (start.middleware && await runMiddleware(start.middleware, request.raw, reply.raw)) return;
        const webRequest = new NodeRequest({ req: request.raw, res: reply.raw, maxRequestBodySize: 1024 * 1024 });
        // Set this on the Web Request, whose headers may be reconstructed from
        // Node rawHeaders rather than Fastify's normalized header object.
        webRequest.headers.set("x-hv-client-ip", request.ip);
        const response = new URL(webRequest.url).pathname.startsWith("/api/auth/")
          ? await runtime.auth.handler(webRequest)
          : await start.fetch(webRequest, { context: { runtime } });
        // Never cache cookie-sensitive HTML, server functions or auth responses.
        response.headers.set("Cache-Control", "no-store");
        await sendNodeResponse(reply.raw, response);
      } catch (error) {
        request.log.error(error, "Web request failed");
        if (reply.raw.headersSent) reply.raw.destroy();
        else {
          reply.raw.writeHead(500, { "Content-Type": "text/plain" });
          reply.raw.end("Internal Server Error");
        }
      }
    });
    await app.ready();
    return app;
  } catch (error) {
    await app.close();
    throw error;
  }
}
