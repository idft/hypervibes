import { mkdir, open, rename, rm } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { join } from "node:path";

const TTL = 30 * 24 * 60 * 60 * 1000;
const MAX_BYTES = 256 * 1024;
const RETRY_MS = 60_000;
const MAX_PENDING = 64;

/** Third-party SVGs are proxied, never trusted: reject anything executable or navigational. */
export function safeSvg(bytes: Buffer): boolean {
  if (!/^\s*(?:<\?xml[^>]*>\s*)?(?:<!--[\s\S]*?-->\s*)*<svg(?:\s|>)/i.test(bytes.toString("utf8"))) return false;
  const text = bytes.toString("utf8");
  return !/<\s*(?:script|foreignObject)\b/i.test(text)
    && !/[\s"'`]on[a-z0-9_-]+\s*=/i.test(text)
    && !/[\s"'`](?:xlink:)?href\s*=\s*(?!["']\s*#)/i.test(text);
}

/** Where a single upstream SVG source is cached on disk, and how its keys behave. */
export interface SvgLogoSource {
  /** Directory under the cache root, forward-slash separated. */
  directory: string;
  /** Arbitrary keys must be rejected before they reach the filesystem or upstream. */
  validate(key: string): boolean;
  url(key: string): string;
  fallback(key: string): Buffer;
}

/** Disposable disk assets, never application or Durable database records. */
export class SvgLogoCache {
  private readonly pending = new Map<string, Promise<Buffer>>();
  private readonly failures = new Map<string, number>();
  private readonly controllers = new Set<AbortController>();
  private readonly waiters: (() => void)[] = [];
  private active = 0;
  private closed = false;

  constructor(private readonly root: string, private readonly source: SvgLogoSource, private readonly fetcher: typeof fetch = fetch) {}

  get(key: string): Promise<Buffer> {
    if (!this.source.validate(key)) return Promise.reject(new Error("Invalid logo key"));
    const existing = this.pending.get(key);
    if (existing) return existing;
    if (this.closed || this.pending.size >= MAX_PENDING) return Promise.resolve(this.source.fallback(key));
    const task = this.load(key).finally(() => this.pending.delete(key));
    this.pending.set(key, task);
    return task;
  }

  async close(): Promise<void> {
    this.closed = true;
    for (const controller of this.controllers) controller.abort();
    await Promise.allSettled(this.pending.values());
  }

  private async load(key: string): Promise<Buffer> {
    // Encoding avoids case collisions and Windows reserved device filenames.
    const path = join(this.root, this.source.directory, `${Buffer.from(key).toString("hex")}.svg`);
    let stale: Buffer | undefined;
    try {
      const file = await open(path, "r");
      try {
        const info = await file.stat();
        if (info.isFile() && info.size <= MAX_BYTES) {
          const bytes = Buffer.alloc(MAX_BYTES + 1);
          const { bytesRead } = await file.read(bytes, 0, bytes.length, 0);
          if (bytesRead <= MAX_BYTES && safeSvg(bytes.subarray(0, bytesRead))) stale = bytes.subarray(0, bytesRead);
          if (stale && Date.now() - info.mtimeMs <= TTL) return stale;
        }
      } finally { await file.close(); }
    } catch { /* Missing, corrupt or inaccessible cache is disposable. */ }
    if (this.closed || (this.failures.get(key) ?? 0) > Date.now()) return stale ?? this.source.fallback(key);
    if (this.active >= 4) await new Promise<void>((resolve) => this.waiters.push(resolve));
    else this.active++;
    const controller = new AbortController();
    this.controllers.add(controller);
    const timer = setTimeout(() => controller.abort(), 10_000);
    let temporary: string | undefined;
    try {
      if (this.closed) throw new Error("Cache closed");
      const response = await this.fetcher(this.source.url(key), { redirect: "error", signal: controller.signal });
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
      if (!safeSvg(bytes)) throw new Error("Invalid SVG");
      // Cache-write failures must not prevent a valid downloaded image being shown.
      try {
        await mkdir(join(this.root, this.source.directory), { recursive: true });
        temporary = `${path}.${randomUUID()}.tmp`;
        const file = await open(temporary, "wx", 0o600);
        try { await file.writeFile(bytes); } finally { await file.close(); }
        await rename(temporary, path);
      } catch { /* Retry persistence on a later request. */ }
      this.failures.delete(key);
      return bytes;
    } catch {
      // Bound negative-cache memory even when arbitrary keys are requested.
      if (this.failures.size >= 1024) this.failures.delete(this.failures.keys().next().value!);
      this.failures.set(key, Date.now() + RETRY_MS);
      return stale ?? this.source.fallback(key);
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