import { mkdir, open, rename, rm } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { isPerpetualLogoSymbol } from "./currency-logo.js";

const TTL = 30 * 24 * 60 * 60 * 1000;
const MAX_BYTES = 256 * 1024;
const RETRY_MS = 60_000;
const MAX_PENDING = 64;
const svg = (bytes: Buffer) => /^\s*(?:<\?xml[^>]*>\s*)?(?:<!--[\s\S]*?-->\s*)*<svg(?:\s|>)/i.test(bytes.toString("utf8"));

export function fallbackCurrencyLogo(symbol: string): Buffer {
  // Only validated symbols may reach the SVG template.
  if (!isPerpetualLogoSymbol(symbol)) throw new Error("Invalid currency symbol");
  return Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="8" fill="#18181b"/><text x="16" y="21" text-anchor="middle" font-family="sans-serif" font-size="${symbol.length <= 3 ? 11 : 8}" textLength="26" lengthAdjust="spacingAndGlyphs" fill="#fafafa">${symbol}</text></svg>`);
}

/** Disposable disk assets, never application or Durable database records. */
export class CurrencyLogoCache {
  private readonly pending = new Map<string, Promise<Buffer>>();
  private readonly failures = new Map<string, number>();
  private readonly controllers = new Set<AbortController>();
  private readonly waiters: (() => void)[] = [];
  private active = 0;
  private closed = false;

  constructor(private readonly root: string, private readonly fetcher: typeof fetch = fetch) {}

  get(symbol: string): Promise<Buffer> {
    if (!isPerpetualLogoSymbol(symbol)) return Promise.reject(new Error("Invalid currency symbol"));
    const existing = this.pending.get(symbol);
    if (existing) return existing;
    if (this.closed || this.pending.size >= MAX_PENDING) return Promise.resolve(fallbackCurrencyLogo(symbol));
    const task = this.load(symbol).finally(() => this.pending.delete(symbol));
    this.pending.set(symbol, task);
    return task;
  }

  async close(): Promise<void> {
    this.closed = true;
    for (const controller of this.controllers) controller.abort();
    await Promise.allSettled(this.pending.values());
  }

  private async load(symbol: string): Promise<Buffer> {
    // Encoding avoids case collisions and Windows reserved device filenames.
    const path = join(this.root, "hyperliquid", "logos", `${Buffer.from(symbol).toString("hex")}.svg`);
    let stale: Buffer | undefined;
    try {
      const file = await open(path, "r");
      try {
        const info = await file.stat();
        if (info.isFile() && info.size <= MAX_BYTES) {
          const bytes = Buffer.alloc(MAX_BYTES + 1);
          const { bytesRead } = await file.read(bytes, 0, bytes.length, 0);
          if (bytesRead <= MAX_BYTES && svg(bytes.subarray(0, bytesRead))) stale = bytes.subarray(0, bytesRead);
          if (stale && Date.now() - info.mtimeMs <= TTL) return stale;
        }
      } finally { await file.close(); }
    } catch { /* Missing, corrupt or inaccessible cache is disposable. */ }
    if (this.closed || (this.failures.get(symbol) ?? 0) > Date.now()) return stale ?? fallbackCurrencyLogo(symbol);
    if (this.active >= 4) await new Promise<void>((resolve) => this.waiters.push(resolve));
    else this.active++;
    const controller = new AbortController();
    this.controllers.add(controller);
    const timer = setTimeout(() => controller.abort(), 10_000);
    let temporary: string | undefined;
    try {
      if (this.closed) throw new Error("Cache closed");
      const response = await this.fetcher(`https://app.hyperliquid.xyz/coins/${encodeURIComponent(symbol)}.svg`, { redirect: "error", signal: controller.signal });
      try {
        const type = response.headers.get("content-type")?.split(";")[0]?.trim().toLowerCase();
        if (!response.ok || (type && !["image/svg+xml", "text/plain", "application/octet-stream"].includes(type))) throw new Error("Invalid logo response");
        if (Number(response.headers.get("content-length")) > MAX_BYTES || !response.body) throw new Error("Invalid logo size");
      } catch (error) { await response.body?.cancel(); throw error; }
      const reader = response.body!.getReader();
      const chunks: Uint8Array[] = [];
      let size = 0;
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          size += value.byteLength;
          if (size > MAX_BYTES) throw new Error("Logo too large");
          chunks.push(value);
        }
      } finally { await reader.cancel(); reader.releaseLock(); }
      const bytes = Buffer.concat(chunks);
      if (!svg(bytes)) throw new Error("Invalid SVG");
      // Cache-write failures must not prevent a valid downloaded image being shown.
      try {
        await mkdir(join(this.root, "hyperliquid", "logos"), { recursive: true });
        temporary = `${path}.${randomUUID()}.tmp`;
        const file = await open(temporary, "wx", 0o600);
        try { await file.writeFile(bytes); } finally { await file.close(); }
        await rename(temporary, path);
      } catch { /* Retry persistence on a later request. */ }
      this.failures.delete(symbol);
      return bytes;
    } catch {
      // Bound negative-cache memory even when arbitrary symbols are requested.
      if (this.failures.size >= 1024) this.failures.delete(this.failures.keys().next().value!);
      this.failures.set(symbol, Date.now() + RETRY_MS);
      return stale ?? fallbackCurrencyLogo(symbol);
    } finally {
      clearTimeout(timer);
      this.controllers.delete(controller);
      if (temporary) await rm(temporary, { force: true }).catch(() => {});
      const next = this.waiters.shift();
      if (next) next();
      else this.active--;
    }
  }
}
