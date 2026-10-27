import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { copyFile, mkdir, mkdtemp, open, rename, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { startScreenshotServer } from "./screenshot-server.mjs";
import { fetchBtcQuote } from "./screenshot-btc-quote.mjs";

const repository = fileURLToPath(new URL("../", import.meta.url));
const defaultOutput = "artifacts/dashboard-screenshot.png";
export const help = `Capture the native dashboard with fictitious accounts and a fresh public BTC-USD quote.
Browser networking stays offline except for local assets. No application server is started.

Usage: pnpm screenshot [--screen dashboard] [--output PATH]

  --screen NAME  Screen to capture (default: dashboard; only dashboard is supported)
  --output PATH  PNG output (default: ${defaultOutput})
  --help, -h     Show this help

The build and loopback server exist only for this capture. Playwright Chromium
must be installed: pnpm exec playwright install chromium
`;

export function parseOptions(args) {
  const options = { screen: "dashboard", output: defaultOutput, help: false };
  for (let index = 0; index < args.length; index++) {
    const argument = args[index];
    if (argument === "--help" || argument === "-h") { options.help = true; continue; }
    const [name, ...parts] = argument.split("=");
    if (name !== "--output" && name !== "--screen") throw new Error(`Unknown option: ${argument}. Use --help.`);
    const value = parts.length ? parts.join("=") : args[++index];
    if (!value || value.startsWith("--")) throw new Error(`${name} requires a value.`);
    options[name.slice(2)] = value;
  }
  if (options.screen !== "dashboard") throw new Error(`Unsupported screen: ${options.screen}. Only dashboard is available.`);
  if (!options.output.toLowerCase().endsWith(".png")) throw new Error("--output must name a .png file.");
  return options;
}

export async function buildScreenshot(directory, signal) {
  signal?.throwIfAborted();
  const vite = fileURLToPath(new URL("bin/vite.js", import.meta.resolve("vite/package.json")));
  await new Promise((resolveBuild, reject) => {
    const child = spawn(process.execPath, [vite, "build", "--configLoader", "native", "--config", resolve(repository, "vite.screenshot.config.mjs"), "--outDir", directory], {
      cwd: repository,
      stdio: ["ignore", "pipe", "pipe"],
      signal,
      // This build does not load the application's .env file.
      env: { ...process.env, NODE_ENV: "production", NODE_DISABLE_COMPILE_CACHE: "1" },
    });
    let diagnostic = "";
    let failure;
    const append = (chunk) => { diagnostic = (diagnostic + chunk.toString()).slice(-16000); };
    child.stdout.on("data", append);
    child.stderr.on("data", append);
    child.on("error", (error) => { failure = error; });
    child.on("close", (code) => {
      if (failure) reject(failure);
      else if (code !== 0) reject(new Error(`Screenshot build failed (${code}):\n${diagnostic}`));
      else resolveBuild();
    });
  });
  signal?.throwIfAborted();
  await mkdir(join(directory, "static"), { recursive: true });
  for (const asset of ["logo.png", "github.svg"]) {
    await copyFile(resolve(repository, "static", asset), join(directory, "static", asset));
  }
  await mkdir(join(directory, "currency"), { recursive: true });
  await copyFile(resolve(repository, "src/screenshot/currency/BTC.svg"), join(directory, "currency/BTC.svg"));
}

export function allowedScreenshotRequest(request, origin, assetPaths) {
  let url;
  try { url = new URL(request.url()); } catch { return false; }
  if (url.origin !== origin || url.username || url.password) return false;
  if (request.method() !== "GET" && request.method() !== "HEAD") return false;
  if (!assetPaths.has(url.pathname) || url.search || url.hash) return false;
  const type = request.resourceType();
  if (url.pathname === "/screenshot") return type === "document" && request.isNavigationRequest();
  return ["script", "stylesheet", "image", "font", "other"].includes(type);
}

export async function waitForScreenshotReady(page) {
  await page.locator('[data-screenshot-ready="true"]').waitFor({ state: "attached" });
  await page.locator("h2").filter({ hasText: /^Dashboard$/ }).waitFor({ state: "attached" });
  await page.locator('section[aria-label="Account overview"]').waitFor({ state: "visible" });
  for (const title of ["Positions", "Open orders"]) {
    await page.locator("h3").filter({ hasText: new RegExp(`^${title}$`) }).waitFor({ state: "visible" });
  }
  for (const name of ["Sub-agents", "Chat"]) {
    await page.locator(`section[aria-label="${name}"]`).waitFor({ state: "visible" });
  }
  await page.evaluate(async () => {
    await document.fonts.ready;
    if (document.fonts.status !== "loaded" || [...document.fonts].some((font) => font.status === "error")) {
      throw new Error("Screenshot fonts did not load");
    }
    await Promise.all([...document.images].map(async (image) => {
      await image.decode();
      if (!image.complete || image.naturalWidth === 0) throw new Error(`Screenshot image did not load: ${image.src}`);
    }));
    await new Promise((resolveFrame) => requestAnimationFrame(() => requestAnimationFrame(resolveFrame)));
    const rows = [...document.querySelectorAll("table tr")];
    if (!rows.length) throw new Error("Screenshot has no native table rows");
    for (const row of rows) {
      const bounds = row.getBoundingClientRect();
      if (!bounds.width || !bounds.height || bounds.top < -1 || bounds.bottom > innerHeight + 1 || bounds.left < -1 || bounds.right > innerWidth + 1) {
        throw new Error(`Screenshot table row is outside the viewport: ${row.textContent}`);
      }
      for (let ancestor = row.parentElement; ancestor; ancestor = ancestor.parentElement) {
        // Root overflow is propagated to the viewport. The fixed app shell does
        // not contribute to the body's clientHeight, so check the viewport above.
        if (ancestor === document.body || ancestor === document.documentElement) continue;
        const style = getComputedStyle(ancestor);
        const clipX = /auto|scroll|hidden|clip/.test(style.overflowX);
        const clipY = /auto|scroll|hidden|clip/.test(style.overflowY);
        if (!clipX && !clipY) continue;
        const rect = ancestor.getBoundingClientRect();
        const left = rect.left + ancestor.clientLeft;
        const top = rect.top + ancestor.clientTop;
        if ((clipX && (bounds.left < left - 1 || bounds.right > left + ancestor.clientWidth + 1))
          || (clipY && (bounds.top < top - 1 || bounds.bottom > top + ancestor.clientHeight + 1))) {
          throw new Error(`Screenshot table row is clipped by ${ancestor.tagName}.${ancestor.className}: ${row.textContent}`);
        }
      }
    }
  });
}

async function launchScreenshotBrowser(temporary) {
  // Playwright can leave a profile directory when executable lookup fails.
  // Keep all launch files inside our owned directory, including that failure.
  // This launcher runs only in the standalone capture process.
  const names = ["TMPDIR", "TMP", "TEMP"];
  const original = names.map((name) => process.env[name]);
  for (const name of names) process.env[name] = temporary;
  try {
    const { chromium } = await import("playwright");
    return await chromium.launch({
      headless: true,
      timeout: 30000,
      artifactsDir: join(temporary, "browser-artifacts"),
    });
  } finally {
    names.forEach((name, index) => {
      if (original[index] === undefined) delete process.env[name];
      else process.env[name] = original[index];
    });
  }
}

// Resource factories let tests exercise cleanup without an installed browser.
export async function captureScreenshot(options, {
  signal,
  fetchQuote = fetchBtcQuote,
  build = buildScreenshot,
  startServer = startScreenshotServer,
  launchBrowser = launchScreenshotBrowser,
} = {}) {
  if (options.screen !== "dashboard") throw new Error(`Unsupported screen: ${options.screen}`);
  const temporary = await mkdtemp(join(tmpdir(), "hypervibes-screenshot-"));
  let server;
  let browser;
  let browserClosing;
  let pendingOutput;
  const closeBrowser = () => {
    if (browser && !browserClosing) browserClosing = browser.close();
    return browserClosing;
  };
  const onAbort = () => { void closeBrowser()?.catch(() => {}); };
  signal?.addEventListener("abort", onAbort, { once: true });
  try {
    signal?.throwIfAborted();
    const directory = join(temporary, "build");
    await build(directory, signal);
    signal?.throwIfAborted();
    // The fixture reads this at runtime. Fetch after the build to keep it fresh.
    const quote = await fetchQuote({ signal });
    signal?.throwIfAborted();
    server = await startServer(directory);
    signal?.throwIfAborted();
    browser = await launchBrowser(temporary);
    signal?.throwIfAborted();
    const context = await browser.newContext({
      viewport: { width: 1600, height: 1400 },
      deviceScaleFactor: 1,
      colorScheme: "dark",
      reducedMotion: "reduce",
      locale: "en-US",
      timezoneId: "UTC",
      serviceWorkers: "block",
      acceptDownloads: false,
    });
    const errors = [];
    await context.route("**/*", async (route) => {
      const request = route.request();
      if (allowedScreenshotRequest(request, server.origin, server.assetPaths)) await route.continue();
      else {
        errors.push(`Blocked request: ${request.method()} ${request.url()}`);
        await route.abort("blockedbyclient");
      }
    });
    await context.routeWebSocket("**/*", (socket) => {
      errors.push(`Blocked WebSocket: ${socket.url()}`);
      socket.close();
    });
    const page = await context.newPage();
    await page.addInitScript((price) => { window.__screenshotBtcPrice = price; }, quote.price);
    page.setDefaultTimeout(30000);
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
    page.on("requestfailed", (request) => errors.push(`Failed request: ${request.url()}`));
    page.on("response", (response) => { if (response.status() >= 400) errors.push(`HTTP ${response.status()}: ${response.url()}`); });
    context.on("page", (extra) => { if (extra !== page) { errors.push("Unexpected screenshot popup"); void extra.close().catch(() => {}); } });
    await page.goto(`${server.origin}/screenshot`, { waitUntil: "networkidle", timeout: 30000 });
    await waitForScreenshotReady(page);
    if (errors.length) throw new Error(`Screenshot validation failed:\n${errors.join("\n")}`);
    signal?.throwIfAborted();
    const png = await page.screenshot({ type: "png", fullPage: true, animations: "disabled", caret: "hide", timeout: 30000 });
    if (errors.length) throw new Error(`Screenshot validation failed:\n${errors.join("\n")}`);
    signal?.throwIfAborted();
    const output = resolve(options.output);
    await mkdir(dirname(output), { recursive: true });
    pendingOutput = join(dirname(output), `.${randomUUID()}.screenshot.tmp`);
    const file = await open(pendingOutput, "wx", 0o600);
    try { await file.writeFile(png); await file.sync(); }
    finally { await file.close(); }
    signal?.throwIfAborted();
    await rename(pendingOutput, output);
    pendingOutput = undefined;
    return output;
  } finally {
    signal?.removeEventListener("abort", onAbort);
    // Each cleanup runs even if an earlier cleanup fails.
    try { await closeBrowser(); }
    finally {
      try { await server?.close(); }
      finally {
        try { if (pendingOutput) await rm(pendingOutput, { force: true }); }
        finally { await rm(temporary, { recursive: true, force: true }); }
      }
    }
  }
}

async function main() {
  const options = parseOptions(process.argv.slice(2));
  if (options.help) { process.stdout.write(help); return; }
  const abort = new AbortController();
  const terminate = (signal) => {
    process.exitCode = signal === "SIGINT" ? 130 : 143;
    abort.abort(new Error(`Screenshot capture stopped by ${signal}`));
  };
  const interrupt = () => terminate("SIGINT");
  const terminateProcess = () => terminate("SIGTERM");
  process.once("SIGINT", interrupt);
  process.once("SIGTERM", terminateProcess);
  const timeout = setTimeout(() => abort.abort(new Error("Screenshot capture exceeded 180 seconds")), 180000);
  try {
    const output = await captureScreenshot(options, {
      signal: abort.signal,
      fetchQuote: async (request) => {
        const quote = await fetchBtcQuote(request);
        process.stdout.write(`BTC-USD ${quote.price} USD | Source: ${quote.source} | Timestamp: ${quote.timestamp}\n`);
        return quote;
      },
    });
    process.stdout.write(`Saved ${output}\n`);
  } finally {
    clearTimeout(timeout);
    process.removeListener("SIGINT", interrupt);
    process.removeListener("SIGTERM", terminateProcess);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode ||= 1;
  });
}
