import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile } from "node:fs/promises";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import { btcQuoteUrl, btcQuoteTimeoutMs, btcQuoteMaxAgeMs, btcQuoteFutureToleranceMs, fetchBtcQuote } from "./screenshot-btc-quote.mjs";
import {
  allowedScreenshotRequest, buildScreenshot, captureScreenshot as captureWithRealQuote, parseOptions,
} from "./capture-screenshot.mjs";
import { canonicalRequestPath, loadScreenshotAssets, startScreenshotServer } from "./screenshot-server.mjs";
import { screenshotBoundariesPlugin } from "../vite.screenshot.config.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));
const timeout = 240000;
const browserTests = process.env.SCREENSHOT_BROWSER_TEST === "1";
const quote = { price: "98765.43", source: btcQuoteUrl, timestamp: "2026-10-27T12:00:00.000Z" };
const fetchQuote = async () => quote;
// Every in-process capture is deterministic unless a test supplies its own factory.
const captureScreenshot = (options, factories = {}) => captureWithRealQuote(options, { fetchQuote, ...factories });
// Stub only the public Node ticker request. The browser keeps its real offline rules.
// This preload is test-only; the CLI has no fixed-price flag or fallback.
const quotePreload = `
  import assert from 'node:assert/strict';
  globalThis.fetch = async (url, options) => {
    assert.equal(url, ${JSON.stringify(btcQuoteUrl)});
    assert.equal(options.method, 'GET');
    assert.equal(options.credentials, 'omit');
    return new Response(JSON.stringify({ price: ${JSON.stringify(quote.price)}, time: new Date().toISOString() }));
  };
`;

function fakeBrowser({ onNavigate, readyError, pageError } = {}) {
  const pendingNavigation = Promise.withResolvers();
  const records = { closed: 0, selectors: [], events: new Map() };
  const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=", "base64");
  const page = {
    addInitScript: async (callback, price) => {
      records.initScript = callback;
      records.price = price;
      const window = {};
      runInNewContext(`(${callback.toString()})(price)`, { window, price });
      records.browserPrice = window.__screenshotBtcPrice;
    },
    setDefaultTimeout() {},
    on: (name, callback) => records.events.set(name, callback),
    locator(selector) {
      records.selectors.push(selector);
      const locator = {
        filter: () => locator,
        waitFor: async () => { if (readyError) throw readyError; },
      };
      return locator;
    },
    goto: async () => {
      assert.equal(records.browserPrice, quote.price, "quote must exist before browser scripts run");
      if (onNavigate) { onNavigate(); await pendingNavigation.promise; }
    },
    evaluate: async () => { if (pageError) records.events.get("pageerror")(pageError); },
    screenshot: async () => png,
  };
  const context = {
    route: async (_pattern, callback) => { records.route = callback; },
    routeWebSocket: async (_pattern, callback) => { records.webSocket = callback; },
    newPage: async () => page,
    on() {},
  };
  const browser = {
    newContext: async (options) => { records.contextOptions = options; return context; },
    close: async () => {
      records.closed++;
      if (onNavigate) pendingNavigation.reject(new Error("Browser closed after cancellation"));
    },
  };
  return { browser, records, png };
}

async function fakeBuild(directory) {
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, "index.html"), "<html>offline</html>");
}

function trackedServer(records) {
  return async (directory) => {
    records.directory = directory;
    const server = await startScreenshotServer(directory);
    records.origin = server.origin;
    return server;
  };
}

async function temporary(run) {
  const directory = await mkdtemp(join(tmpdir(), "hypervibes-screenshot-test-"));
  try { return await run(directory); }
  finally { await rm(directory, { recursive: true, force: true }); }
}

function rawRequest(origin, path, method = "GET", headers = {}) {
  return new Promise((resolveRequest, reject) => {
    const url = new URL(origin);
    const req = request({ hostname: url.hostname, port: url.port, path, method, headers }, (response) => {
      const chunks = [];
      response.on("data", (chunk) => chunks.push(chunk));
      response.on("end", () => resolveRequest({ status: response.statusCode, headers: response.headers, body: Buffer.concat(chunks).toString() }));
    });
    req.on("error", reject);
    req.end();
  });
}

function screenshotProcess(args, environment = {}) {
  const preload = `data:text/javascript,${encodeURIComponent(quotePreload)}`;
  const child = spawn(process.execPath, ["--import", preload, resolve(repository, "scripts/capture-screenshot.mjs"), ...args], {
    cwd: repository, env: { ...process.env, ...environment }, stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  child.stdout.on("data", (chunk) => { output += chunk; });
  child.stderr.on("data", (chunk) => { output += chunk; });
  const completion = new Promise((resolveProcess, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolveProcess({ code, signal, output }));
  });
  return { child, completion };
}

test("public quote fetch uses only the credential-free Coinbase GET and preserves its decimal price", { timeout }, async () => {
  const now = Date.parse(quote.timestamp);
  const controller = new AbortController();
  let calls = 0;
  assert.deepEqual(await fetchBtcQuote({
    signal: controller.signal, now: () => now,
    fetch: async (url, options) => {
      calls++;
      assert.equal(url, btcQuoteUrl);
      assert.equal(options.method, "GET");
      assert.equal(options.credentials, "omit");
      assert.equal(options.cache, "no-store");
      assert.equal(options.redirect, "error");
      assert.deepEqual(options.headers, { Accept: "application/json" });
      assert.equal(options.signal.aborted, false);
      return new Response(JSON.stringify({ price: quote.price, time: quote.timestamp }));
    },
  }), quote);
  assert.equal(calls, 1);
});

test("public quote rejects invalid prices, invalid timestamps, stale trades, and future trades", { timeout }, async () => {
  const now = Date.parse(quote.timestamp);
  const invalidPrices = [undefined, null, 123, "", "0", "0.00000000", "-1", "NaN", "Infinity", "1e5", " 123", "123 ", ".5", "01.2", "1.", "1.000000001", "99999999999999999", "999.99999999", "1000000.00000001"];
  for (const price of invalidPrices) {
    await assert.rejects(fetchBtcQuote({ now: () => now, fetch: async () => new Response(JSON.stringify({ price, time: quote.timestamp })) }), /invalid price/);
  }
  for (const price of ["1000", "1000.00000001", "98765.12345678", "1000000.00000000"]) {
    const result = await fetchBtcQuote({ now: () => now, fetch: async () => new Response(JSON.stringify({ price, time: quote.timestamp })) });
    assert.equal(result.price, price);
  }
  for (const time of [undefined, null, 123, "", "yesterday", "2026-02-30T12:00:00Z", "2026-10-27", "2026-10-27T12:00:00", "2026-10-27T12:00:00+00:00"]) {
    await assert.rejects(fetchBtcQuote({ now: () => now, fetch: async () => new Response(JSON.stringify({ price: quote.price, time })) }), /invalid timestamp/);
  }
  for (const offset of [-btcQuoteMaxAgeMs - 1, btcQuoteFutureToleranceMs + 1]) {
    await assert.rejects(fetchBtcQuote({ now: () => now, fetch: async () => new Response(JSON.stringify({ price: quote.price, time: new Date(now + offset).toISOString() })) }), /not fresh/);
  }
  // Check freshness after the body arrives, not when the request starts.
  let clock = now;
  await assert.rejects(fetchBtcQuote({ now: () => clock, fetch: async () => ({
    ok: true,
    json: async () => { clock += btcQuoteMaxAgeMs + 1; return { price: quote.price, time: quote.timestamp }; },
  }) }), /not fresh/);
});

test("public quote rejects HTTP, malformed JSON, and network errors without retry", { timeout }, async () => {
  for (const status of [301, 401, 429, 500]) {
    let calls = 0;
    await assert.rejects(fetchBtcQuote({ fetch: async () => { calls++; return new Response("error", { status }); } }), new RegExp(`HTTP ${status}`));
    assert.equal(calls, 1);
  }
  await assert.rejects(fetchBtcQuote({ fetch: async () => new Response("not JSON") }), SyntaxError);
  await assert.rejects(fetchBtcQuote({ fetch: async () => { throw new Error("offline"); } }), /offline/);
});

test("the 10-second quote timeout bounds both headers and response-body reads", { timeout }, async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  assert.equal(btcQuoteTimeoutMs, 10000);
  for (const stalledBody of [false, true]) {
    let requestSignal;
    let bodyStarted = false;
    const pending = fetchBtcQuote({ fetch: async (_url, options) => {
      requestSignal = options.signal;
      if (!stalledBody) return new Promise(() => {});
      return { ok: true, json: () => { bodyStarted = true; return new Promise(() => {}); } };
    } });
    const rejected = assert.rejects(pending, /timed out after 10 seconds/);
    await Promise.resolve();
    assert.equal(bodyStarted, stalledBody);
    context.mock.timers.tick(btcQuoteTimeoutMs - 1);
    assert.equal(requestSignal.aborted, false);
    context.mock.timers.tick(1);
    assert.equal(requestSignal.aborted, true);
    await rejected;
  }
});

test("quote cancellation propagates before fetch and while headers or the body are pending", { timeout }, async () => {
  const reason = new Error("caller cancelled quote");
  await assert.rejects(fetchBtcQuote({ signal: AbortSignal.abort(reason), fetch: async () => assert.fail("must not fetch") }), (error) => error === reason);
  for (const stalledBody of [false, true]) {
    const controller = new AbortController();
    let requestSignal;
    const pending = fetchBtcQuote({ signal: controller.signal, fetch: async (_url, options) => {
      requestSignal = options.signal;
      return stalledBody ? { ok: true, json: () => new Promise(() => {}) } : new Promise(() => {});
    } });
    const rejected = assert.rejects(pending, (error) => error === reason);
    await Promise.resolve();
    controller.abort(reason);
    await rejected;
    assert.equal(requestSignal.aborted, true);
  }
});

test("invalid or unavailable quotes remove the completed build without writing a PNG", { timeout }, async () => temporary(async (directory) => {
  const output = join(directory, "dashboard.png");
  const missingOutput = join(directory, "never.png");
  await writeFile(output, "previous screenshot");
  for (const getter of [
    async () => { throw new Error("quote unavailable"); },
    (options) => fetchBtcQuote({ ...options, fetch: async () => new Response(JSON.stringify({ price: "0", time: quote.timestamp })) }),
    (options) => fetchBtcQuote({ ...options, now: () => Date.parse(quote.timestamp) + btcQuoteMaxAgeMs + 1, fetch: async () => new Response(JSON.stringify({ price: quote.price, time: quote.timestamp })) }),
  ]) {
    for (const target of [output, missingOutput]) {
      let buildDirectory;
      await assert.rejects(captureScreenshot({ screen: "dashboard", output: target }, {
        fetchQuote: getter,
        build: async (path) => { buildDirectory = path; await fakeBuild(path); },
        startServer: async () => assert.fail("must not start a server without a quote"),
        launchBrowser: async () => assert.fail("must not launch a browser without a quote"),
      }), /quote unavailable|invalid price|not fresh/);
      assert.ok(buildDirectory, "the static build must run before the quote fetch");
      await assert.rejects(readdir(buildDirectory), { code: "ENOENT" });
      assert.equal(await readFile(output, "utf8"), "previous screenshot");
      await assert.rejects(readFile(missingOutput), { code: "ENOENT" });
    }
  }
  assert.deepEqual(await readdir(directory), ["dashboard.png"]);
}));

test("cancellation during the public quote removes the completed build without a PNG", { timeout }, async () => temporary(async (directory) => {
  const controller = new AbortController();
  let buildDirectory;
  await assert.rejects(captureScreenshot({ screen: "dashboard", output: join(directory, "never.png") }, {
    signal: controller.signal,
    fetchQuote: (options) => fetchBtcQuote({ ...options, fetch: async (_url, { signal }) => {
      queueMicrotask(() => controller.abort(new Error("cancel quote after build")));
      assert.equal(signal.aborted, false);
      return new Promise(() => {});
    } }),
    build: async (path) => { buildDirectory = path; await fakeBuild(path); },
    startServer: async () => assert.fail("must not start a server after quote cancellation"),
    launchBrowser: async () => assert.fail("must not launch a browser after quote cancellation"),
  }), /cancel quote after build/);
  assert.ok(buildDirectory);
  await assert.rejects(readdir(buildDirectory), { code: "ENOENT" });
  assert.deepEqual(await readdir(directory), []);
}));

test("each capture fetches exactly once after building and before browser launch, and forwards caller cancellation", { timeout }, async () => temporary(async (directory) => {
  const events = [];
  const controller = new AbortController();
  for (let index = 0; index < 2; index++) {
    const fixture = fakeBrowser();
    await captureScreenshot({ screen: "dashboard", output: join(directory, `${index}.png`) }, {
      signal: controller.signal,
      fetchQuote: async ({ signal }) => { assert.equal(signal, controller.signal); events.push("quote"); return quote; },
      build: async (path) => { await fakeBuild(path); events.push("build complete"); },
      launchBrowser: async () => { events.push("browser"); return fixture.browser; },
    });
    assert.equal(fixture.records.browserPrice, quote.price);
  }
  assert.deepEqual(events, ["build complete", "quote", "browser", "build complete", "quote", "browser"]);
}));

test("CLI defaults, help, screen selection, and output validation", { timeout }, async () => {
  assert.deepEqual(parseOptions([]), { screen: "dashboard", output: "artifacts/dashboard-screenshot.png", help: false });
  assert.equal(parseOptions(["--screen", "dashboard", "--output=example.png"]).output, "example.png");
  assert.equal(parseOptions(["-h"]).help, true);
  for (const arguments_ of [["--screen", "chart"], ["--output"], ["--output=x.jpg"], ["--port=3000"], ["--output", "--screen"]]) {
    assert.throws(() => parseOptions(arguments_));
  }
  const help = await screenshotProcess(["--help"]).completion;
  assert.equal(help.code, 0, help.output);
  assert.match(help.output, /--screen/);
  assert.match(help.output, /--output/);
  const unsupported = await screenshotProcess(["--screen", "chart"]).completion;
  assert.equal(unsupported.code, 1);
  assert.match(unsupported.output, /Only dashboard is available/);
});

test("pre-resolution boundaries replace JS and TS imports before runtime code can load", { timeout }, () => {
  const plugin = screenshotBoundariesPlugin();
  const importer = resolve(repository, "src/components/dashboard-chat.tsx");
  for (const name of ["agents", "subagents", "indicators", "auth-client"]) {
    for (const extension of ["js", "ts"]) {
      assert.equal(plugin.resolveId(`../web/${name}.${extension}`, importer), resolve(repository, "src/screenshot/screenshot-boundaries.ts"));
      assert.equal(plugin.resolveId(resolve(repository, `src/web/${name}.${extension}`), importer), resolve(repository, "src/screenshot/screenshot-boundaries.ts"));
    }
  }
  for (const extension of ["js", "tsx"]) {
    assert.equal(plugin.resolveId(`./agent-events.${extension}`, importer), resolve(repository, "src/screenshot/screenshot-events.tsx"));
  }
  assert.equal(plugin.resolveId("../web/agents.js?test", importer), resolve(repository, "src/screenshot/screenshot-boundaries.ts"));
  assert.equal(plugin.resolveId("../hyperliquid/decimal.js", importer), null);
  assert.equal(plugin.resolveId("../web/random-uuid.js", importer), null);
  for (const specifier of ["node:sqlite", "fs", "node:http", "@tanstack/react-start", "@earendil-works/pi-durable", "@nktkas/hyperliquid", "better-auth/react", "../runtime.js", "../main.js", "../db/database.js", "../web/app.js", "../hyperliquid/service.js", "../agents/agent-service.js"]) {
    assert.throws(() => plugin.resolveId(specifier, importer), /rejects/);
  }
  assert.throws(() => plugin.load(resolve(repository, "src/runtime.ts")), /backend-only/);
  assert.throws(() => plugin.configResolved({ root: repository, build: { outDir: ".build" } }), /temporary/);
});

test("the network allowlist blocks non-local, API, write, socket, and alternate document requests", { timeout }, () => {
  const origin = "http://127.0.0.1:49152";
  const paths = new Set(["/screenshot", "/assets/app.js", "/currency/BTC.svg"]);
  const requestFor = (path, method = "GET", type = "script", navigation = false) => ({
    url: () => path.startsWith("/") ? `${origin}${path}` : path,
    method: () => method, resourceType: () => type, isNavigationRequest: () => navigation,
  });
  assert.equal(allowedScreenshotRequest(requestFor("/screenshot", "GET", "document", true), origin, paths), true);
  assert.equal(allowedScreenshotRequest(requestFor("/assets/app.js"), origin, paths), true);
  assert.equal(allowedScreenshotRequest(requestFor("/currency/BTC.svg", "GET", "image"), origin, paths), true);
  for (const req of [
    requestFor("https://example.com/assets/app.js"), requestFor("http://127.0.0.1:3000/assets/app.js"),
    requestFor("ws://127.0.0.1:49152/assets/app.js", "GET", "websocket"),
    requestFor("/api/agents"), requestFor("/agents"), requestFor("/assets/app.js", "POST"),
    requestFor("/assets/app.js", "PUT"), requestFor("/assets/app.js", "DELETE"),
    requestFor("/assets/app.js", "GET", "fetch"), requestFor("/assets/app.js", "GET", "xhr"),
    requestFor("/assets/app.js", "GET", "document", true), requestFor("/assets/app.js?token=secret"),
    requestFor("http://user:password@127.0.0.1:49152/assets/app.js"),
  ]) assert.equal(allowedScreenshotRequest(req, origin, paths), false, req.url());
});

test("temporary server serves only canonical built assets and the screenshot document", { timeout }, async () => temporary(async (directory) => {
  await mkdir(join(directory, "assets"));
  await writeFile(join(directory, "index.html"), "<html>Offline screenshot</html>");
  await writeFile(join(directory, "assets/app.js"), "export {};");
  const server = await startScreenshotServer(directory);
  try {
    assert.equal(new URL(server.origin).hostname, "127.0.0.1");
    assert.notEqual(new URL(server.origin).port, "3000");
    const document = await rawRequest(server.origin, "/screenshot");
    assert.equal(document.status, 200);
    assert.match(document.headers["content-security-policy"], /connect-src 'none'/);
    assert.match(document.headers["content-security-policy"], /worker-src 'none'/);
    assert.equal((await rawRequest(server.origin, "/assets/app.js")).status, 200);
    assert.equal((await rawRequest(server.origin, "/screenshot", "HEAD")).body, "");
    assert.equal((await rawRequest(server.origin, "/screenshot", "POST")).status, 405);
    await assert.rejects(rawRequest(server.origin, "/screenshot", "GET", { Connection: "Upgrade", Upgrade: "websocket" }));
    for (const path of ["/", "/agents", "/login", "/healthz", "/api/agents", "/index.html", "/src/screenshot/index.html", "/.env", "/assets/../index.html", "/assets/%2e%2e/index.html", "/assets//app.js", "//assets/app.js", "/assets\\app.js"]) {
      assert.equal((await rawRequest(server.origin, path)).status, 404, path);
    }
    assert.equal((await rawRequest(server.origin, "/screenshot", "GET", { Host: "example.com" })).status, 403);
    assert.equal((await rawRequest(server.origin, "/screenshot", "GET", { Cookie: "session=secret" })).status, 403);
    assert.equal((await rawRequest(server.origin, "/screenshot", "GET", { Authorization: "Bearer secret" })).status, 403);
    for (const path of ["/%2e%2e/x", "/assets/../x", "/assets//x", "//example.com", "/assets\\x"]) assert.equal(canonicalRequestPath(path), null);
  } finally { await server.close(); }
  await assert.rejects(fetch(`${server.origin}/screenshot`));
}));

test("asset loading rejects source files and symlinks", { timeout }, async () => temporary(async (directory) => {
  await writeFile(join(directory, "index.html"), "offline");
  await writeFile(join(directory, "runtime.ts"), "forbidden");
  await assert.rejects(loadScreenshotAssets(directory), /Unexpected/);
  await rm(join(directory, "runtime.ts"));
  await symlink(join(directory, "index.html"), join(directory, "leak.html"));
  await assert.rejects(loadScreenshotAssets(directory), /symlinks/);
}));

test("the standalone Vite build contains only the browser fixture and required assets", { timeout }, async () => temporary(async (directory) => {
  const build = join(directory, "build");
  await buildScreenshot(build);
  const assets = await loadScreenshotAssets(build);
  assert.ok(assets.has("/screenshot"));
  assert.ok(assets.has("/currency/BTC.svg"));
  assert.deepEqual((await readdir(join(build, "static"))).sort(), ["github.svg", "logo.png"]);
  assert.ok([...assets.keys()].some((path) => path.startsWith("/assets/") && path.endsWith(".js")));
  assert.ok(![...assets.keys()].some((path) => /server|runtime|\.map$/.test(path)));
  const html = assets.get("/screenshot").body.toString();
  assert.doesNotMatch(html, /https?:\/\//);
}));

test("an aborted capture removes its temporary build without starting a browser", { timeout }, async () => temporary(async (directory) => {
  const signal = AbortSignal.abort(new Error("test cancellation"));
  await assert.rejects(captureScreenshot({ screen: "dashboard", output: join(directory, "never.png") }, { signal }), /test cancellation/);
  assert.deepEqual(await readdir(directory), []);
}));

test("optional real Chromium capture creates a PNG atomically and removes its temporary server and files", { timeout, skip: !browserTests && "Set SCREENSHOT_BROWSER_TEST=1 to run the real Chromium capture" }, async () => temporary(async (directory) => {
  const scratch = join(directory, "scratch");
  await mkdir(scratch);
  const output = join(directory, "nested", "dashboard.png");
  await mkdir(join(directory, "nested"));
  await writeFile(output, "previous screenshot");
  const result = await screenshotProcess(["--screen", "dashboard", "--output", output], { TMPDIR: scratch, TMP: scratch, TEMP: scratch }).completion;
  assert.equal(result.code, 0, result.output);
  assert.ok(result.output.includes(`BTC-USD ${quote.price} USD | Source: ${btcQuoteUrl} | Timestamp: `), result.output);
  assert.match(result.output, /Timestamp: \d{4}-\d{2}-\d{2}T/);
  const png = await readFile(output);
  assert.equal(png.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
  assert.equal(png.readUInt32BE(16), 1600);
  assert.ok(png.readUInt32BE(20) >= 1400);
  assert.deepEqual(await readdir(scratch), []);
  assert.deepEqual(await readdir(join(directory, "nested")), ["dashboard.png"]);

  // Fail after rendering. Existing output data must remain unchanged.
  const failure = await screenshotProcess(["--output", join(output, "failure.png")], { TMPDIR: scratch, TMP: scratch, TEMP: scratch }).completion;
  assert.equal(failure.code, 1, failure.output);
  assert.match(failure.output, /EEXIST|ENOTDIR/);
  assert.deepEqual(await readFile(output), png);
  assert.deepEqual(await readdir(scratch), []);
}));

test("optional real Chromium rejects clipped native table rows", { timeout, skip: !browserTests && "Set SCREENSHOT_BROWSER_TEST=1 to run the real Chromium capture" }, async () => temporary(async (directory) => {
  const { chromium } = await import("playwright");
  const server = {};
  const output = join(directory, "dashboard.png");
  await writeFile(output, "previous screenshot");
  await assert.rejects(captureScreenshot({ screen: "dashboard", output }, {
    startServer: trackedServer(server),
    launchBrowser: async () => {
      const browser = await chromium.launch({ headless: true, timeout: 30000 });
      return {
        newContext: (options) => browser.newContext({ ...options, viewport: { width: 1600, height: 1100 } }),
        close: () => browser.close(),
      };
    },
  }), /Screenshot table row is clipped/);
  assert.equal(await readFile(output, "utf8"), "previous screenshot");
  await assert.rejects(fetch(`${server.origin}/screenshot`));
  await assert.rejects(readdir(server.directory), { code: "ENOENT" });
  assert.deepEqual(await readdir(directory), ["dashboard.png"]);
}));

test("CLI browser startup failure needs no Chromium and preserves existing output", { timeout }, async () => temporary(async (directory) => {
  const scratch = join(directory, "scratch");
  const browsers = join(directory, "empty-browsers");
  await mkdir(scratch);
  await mkdir(browsers);
  const output = join(directory, "dashboard.png");
  await writeFile(output, "previous screenshot");
  const result = await screenshotProcess(["--output", output], {
    TMPDIR: scratch, TMP: scratch, TEMP: scratch, PLAYWRIGHT_BROWSERS_PATH: browsers,
  }).completion;
  assert.equal(result.code, 1, result.output);
  assert.match(result.output, /Executable doesn't exist/);
  assert.equal(await readFile(output, "utf8"), "previous screenshot");
  assert.deepEqual(await readdir(scratch), []);
  assert.deepEqual(await readdir(browsers), []);
}));

test("an injected build failure removes incomplete temporary files", { timeout }, async () => temporary(async (directory) => {
  let buildDirectory;
  await assert.rejects(captureScreenshot({ screen: "dashboard", output: join(directory, "never.png") }, {
    build: async (path) => {
      buildDirectory = path;
      await fakeBuild(path);
      throw new Error("injected build failure");
    },
    fetchQuote: async () => assert.fail("must not fetch a quote after a failed build"),
    launchBrowser: async () => { throw new Error("Browser must not start"); },
  }), /injected build failure/);
  await assert.rejects(readdir(buildDirectory), { code: "ENOENT" });
  assert.deepEqual(await readdir(directory), []);
}));

test("an injected browser launch failure closes the real listener and removes the real build", { timeout }, async () => temporary(async (directory) => {
  const records = {};
  await assert.rejects(captureScreenshot({ screen: "dashboard", output: join(directory, "never.png") }, {
    startServer: trackedServer(records),
    launchBrowser: async () => { throw new Error("injected browser launch failure"); },
  }), /injected browser launch failure/);
  await assert.rejects(fetch(`${records.origin}/screenshot`));
  await assert.rejects(readdir(records.directory), { code: "ENOENT" });
  assert.deepEqual(await readdir(directory), []);
}));

test("injected browser success writes atomically and closes every resource", { timeout }, async () => temporary(async (directory) => {
  const fixture = fakeBrowser();
  const server = {};
  const output = join(directory, "dashboard.png");
  await writeFile(output, "previous screenshot");
  assert.equal(await captureScreenshot({ screen: "dashboard", output }, {
    build: fakeBuild, startServer: trackedServer(server), launchBrowser: async () => fixture.browser,
  }), output);
  assert.deepEqual(await readFile(output), fixture.png);
  assert.equal(fixture.records.closed, 1);
  assert.equal(fixture.records.price, quote.price);
  assert.equal(fixture.records.browserPrice, quote.price);
  assert.equal(fixture.records.contextOptions.serviceWorkers, "block");
  assert.deepEqual(fixture.records.contextOptions.viewport, { width: 1600, height: 1400 });
  assert.ok(fixture.records.selectors.includes('[data-screenshot-ready="true"]'));
  for (const name of ["Account overview", "Sub-agents", "Chat"]) {
    assert.ok(fixture.records.selectors.includes(`section[aria-label="${name}"]`));
  }
  assert.equal(typeof fixture.records.route, "function");
  assert.equal(typeof fixture.records.webSocket, "function");
  await assert.rejects(fetch(`${server.origin}/screenshot`));
  await assert.rejects(readdir(server.directory), { code: "ENOENT" });
  assert.deepEqual(await readdir(directory), ["dashboard.png"]);
}));

for (const reason of ["readiness", "page error", "output"]) {
  test(`injected ${reason} failure closes resources and preserves existing output`, { timeout }, async () => temporary(async (directory) => {
    const fixture = fakeBrowser({
      readyError: reason === "readiness" ? new Error("injected readiness failure") : undefined,
      pageError: reason === "page error" ? new Error("injected page error") : undefined,
    });
    const server = {};
    const output = join(directory, "dashboard.png");
    await writeFile(output, "previous screenshot");
    const captureOutput = reason === "output" ? join(output, "failure.png") : output;
    await assert.rejects(captureScreenshot({ screen: "dashboard", output: captureOutput }, {
      build: fakeBuild, startServer: trackedServer(server), launchBrowser: async () => fixture.browser,
    }), reason === "output" ? /EEXIST|ENOTDIR/ : /injected/);
    assert.equal(fixture.records.closed, 1);
    assert.equal(await readFile(output, "utf8"), "previous screenshot");
    await assert.rejects(fetch(`${server.origin}/screenshot`));
    await assert.rejects(readdir(server.directory), { code: "ENOENT" });
    assert.deepEqual(await readdir(directory), ["dashboard.png"]);
  }));
}

test("an abort during navigation closes the browser, listener, and temporary files", { timeout }, async () => temporary(async (directory) => {
  const abort = new AbortController();
  const fixture = fakeBrowser({ onNavigate: () => abort.abort(new Error("injected active cancellation")) });
  const server = {};
  await assert.rejects(captureScreenshot({ screen: "dashboard", output: join(directory, "never.png") }, {
    signal: abort.signal, build: fakeBuild, startServer: trackedServer(server), launchBrowser: async () => fixture.browser,
  }), /cancellation/);
  assert.equal(fixture.records.closed, 1);
  await assert.rejects(fetch(`${server.origin}/screenshot`));
  await assert.rejects(readdir(server.directory), { code: "ENOENT" });
  assert.deepEqual(await readdir(directory), []);
}));

for (const signal of ["SIGINT", "SIGTERM"]) {
  test(`${signal} removes the temporary build and exits without output`, { timeout }, async () => temporary(async (directory) => {
    const scratch = join(directory, "scratch");
    await mkdir(scratch);
    const output = join(directory, "never.png");
    const process_ = screenshotProcess(["--output", output], { TMPDIR: scratch, TMP: scratch, TEMP: scratch });
    try {
      for (let attempt = 0; ; attempt++) {
        if ((await readdir(scratch)).some((name) => name.startsWith("hypervibes-screenshot-"))) break;
        if (attempt > 1000) throw new Error("Screenshot process did not create a temporary build");
        await delay(5);
      }
      process_.child.kill(signal);
      const result = await process_.completion;
      assert.equal(result.code, signal === "SIGINT" ? 130 : 143, result.output);
      assert.deepEqual(await readdir(scratch), []);
      await assert.rejects(readFile(output), { code: "ENOENT" });
    } finally {
      if (process_.child.exitCode === null) process_.child.kill("SIGTERM");
      await process_.completion;
    }
  }));
}
