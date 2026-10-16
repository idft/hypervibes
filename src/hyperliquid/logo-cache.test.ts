import { mkdtemp, readFile, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CurrencyLogoCache } from "./logo-cache.js";
import { currencyLogoUrl } from "./currency-logo.js";

const image = '<svg xmlns="http://www.w3.org/2000/svg"><circle r="1"/></svg>';
const response = () => new Response(image, { headers: { "Content-Type": "image/svg+xml" } });
describe("perpetual currency logo cache", () => {
  let directory: string;
  let cache: CurrencyLogoCache;
  const fetcher = vi.fn<typeof fetch>();
  const path = (symbol: string) => join(directory, "hyperliquid", "logos", `${Buffer.from(symbol).toString("hex")}.svg`);
  beforeEach(async () => {
    directory = await mkdtemp(join(tmpdir(), "hypervibes-logos-"));
    fetcher.mockReset().mockImplementation(async () => response());
    cache = new CurrencyLogoCache(directory, fetcher);
  });
  afterEach(async () => { await cache.close(); await rm(directory, { recursive: true, force: true }); vi.useRealTimers(); });

  it("deduplicates downloads and persists across owners, preserving symbol case", async () => {
    const result = await Promise.all([cache.get("BTC"), cache.get("BTC")]);
    expect(result.map((bytes) => bytes.toString())).toEqual([image, image]);
    expect(fetcher).toHaveBeenCalledOnce();
    expect(fetcher.mock.calls[0]?.[0]).toBe("https://app.hyperliquid.xyz/coins/BTC.svg");
    expect(fetcher.mock.calls[0]?.[1]?.redirect).toBe("error");
    expect((await readFile(path("BTC"))).toString()).toBe(image);
    await cache.close();
    cache = new CurrencyLogoCache(directory, fetcher);
    expect((await cache.get("BTC")).toString()).toBe(image);
    expect(fetcher).toHaveBeenCalledOnce();
    await cache.get("btc");
    expect(fetcher).toHaveBeenCalledTimes(2);
  });
  it("refreshes after 30 days and serves stale bytes on failure", async () => {
    await cache.get("BTC");
    const old = new Date(Date.now() - 31 * 86_400_000);
    await utimes(path("BTC"), old, old);
    fetcher.mockRejectedValueOnce(new Error("offline"));
    expect((await cache.get("BTC")).toString()).toBe(image);
    expect((await cache.get("BTC")).toString()).toBe(image);
    expect(fetcher).toHaveBeenCalledTimes(2);
  });
  it.each([
    () => new Response("missing", { status: 404 }),
    () => new Response("<html>not an image</html>", { headers: { "Content-Type": "image/svg+xml" } }),
    () => new Response(image, { headers: { "Content-Type": "text/html" } }),
    () => new Response(image, { headers: { "Content-Length": "999999" } }),
    () => new Response("<svg>" + "x".repeat(256 * 1024)),
  ])("does not cache invalid upstream responses and backs off retries", async (invalid) => {
    fetcher.mockImplementation(async () => invalid());
    expect((await cache.get("XYZ")).toString()).toContain(">XYZ</text>");
    await expect(readFile(path("XYZ"))).rejects.toThrow();
    await cache.get("XYZ");
    expect(fetcher).toHaveBeenCalledOnce();
  });
  it("retries missing images after a short negative-cache interval", async () => {
    fetcher.mockRejectedValueOnce(new Error("offline"));
    await cache.get("BTC");
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(Date.now() + 61_000);
    expect((await cache.get("BTC")).toString()).toBe(image);
    expect(fetcher).toHaveBeenCalledTimes(2);
  });
  it("replaces corrupted disk entries and tolerates an unwritable cache", async () => {
    await cache.get("BTC");
    await writeFile(path("BTC"), "<html>bad</html>");
    expect((await cache.get("BTC")).toString()).toBe(image);
    const fileRoot = join(directory, "not-a-directory");
    await writeFile(fileRoot, "file");
    await cache.close();
    cache = new CurrencyLogoCache(fileRoot, fetcher);
    expect((await cache.get("ETH")).toString()).toBe(image);
  });
  it.each(["../BTC", "@107", "BTC/USDC", "BTC.svg", '<svg onload="x">', "", "x".repeat(65), "a\\b"])("rejects unsafe or spot identifiers: %s", async (symbol) => {
    expect(currencyLogoUrl(symbol)).toBeNull();
    await expect(cache.get(symbol)).rejects.toThrow("Invalid");
    expect(fetcher).not.toHaveBeenCalled();
  });
  it("caps concurrent downloads at four and cancels them on shutdown", async () => {
    fetcher.mockImplementation((_url, options) => new Promise((_resolve, reject) => {
      options?.signal?.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
    }));
    const tasks = Array.from({ length: 8 }, (_, i) => cache.get(`COIN${i}`));
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(4));
    await cache.close();
    expect(await Promise.all(tasks)).toHaveLength(8);
    expect(fetcher).toHaveBeenCalledTimes(4);
    await cache.get("OTHER");
    expect(fetcher).toHaveBeenCalledTimes(4);
  });
  it("times out hung downloads", async () => {
    vi.useFakeTimers();
    fetcher.mockImplementation((_url, options) => new Promise((_resolve, reject) => {
      options?.signal?.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
    }));
    const task = cache.get("BTC");
    await vi.waitFor(() => expect(fetcher).toHaveBeenCalledOnce());
    await vi.advanceTimersByTimeAsync(10_001);
    expect((await task).toString()).toContain(">BTC</text>");
  });
});
