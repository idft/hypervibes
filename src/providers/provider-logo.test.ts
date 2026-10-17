import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderLogoCache, fallbackProviderLogo, isProviderSlug } from "./provider-logo.js";
import { providerLogoUrl } from "./provider-logo-url.js";

const image = '<svg xmlns="http://www.w3.org/2000/svg"><circle r="1"/></svg>';
describe("provider logo cache", () => {
  let directory: string;
  let cache: ProviderLogoCache;
  const fetcher = vi.fn<typeof fetch>();
  const path = (provider: string) => join(directory, "model-catalog", "logos", `${Buffer.from(provider).toString("hex")}.svg`);
  beforeEach(async () => {
    directory = await mkdtemp(join(tmpdir(), "hypervibes-provider-logos-"));
    fetcher.mockReset().mockImplementation(async () => new Response(image, { headers: { "Content-Type": "image/svg+xml" } }));
    cache = new ProviderLogoCache(directory, fetcher);
  });
  afterEach(async () => { await cache.close(); await rm(directory, { recursive: true, force: true }); });

  it("fetches models.dev logos, persists them and serves the same bytes later", async () => {
    expect((await cache.get("anthropic")).toString()).toBe(image);
    expect(fetcher).toHaveBeenCalledOnce();
    expect(fetcher.mock.calls[0]?.[0]).toBe("https://models.dev/logos/anthropic.svg");
    expect((await readFile(path("anthropic"))).toString()).toBe(image);
    fetcher.mockClear();
    expect((await cache.get("anthropic")).toString()).toBe(image);
    expect(fetcher).not.toHaveBeenCalled();
  });
  it.each([
    () => new Response(image.replace("/>", ' onload="x"/>'), { headers: { "Content-Type": "image/svg+xml" } }),
    () => new Response('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>', { headers: { "Content-Type": "image/svg+xml" } }),
    () => new Response('<svg xmlns="http://www.w3.org/2000/svg"><foreignObject>hi</foreignObject></svg>', { headers: { "Content-Type": "image/svg+xml" } }),
    () => new Response('<svg xmlns="http://www.w3.org/2000/svg"><use href="https://evil.example/x.svg"/></svg>', { headers: { "Content-Type": "image/svg+xml" } }),
    () => new Response(image, { headers: { "Content-Type": "text/html" } }),
    () => new Response("missing", { status: 404 }),
  ])("rejects unsafe or invalid upstream bodies and serves the fallback", async (invalid) => {
    fetcher.mockImplementation(async () => invalid());
    const result = await cache.get("openai");
    expect(result.toString()).toContain('aria-label="openai"');
    await expect(readFile(path("openai"))).rejects.toThrow();
  });
  it("accepts local fragment references in proxied logos", async () => {
    const gradient = '<svg xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="g"><stop/></linearGradient></defs><circle fill="url(#g)"/></svg>';
    fetcher.mockImplementation(async () => new Response(gradient, { headers: { "Content-Type": "image/svg+xml" } }));
    expect((await cache.get("anthropic")).toString()).toBe(gradient);
  });
  it("renders the initial-letter fallback for validated providers only", () => {
    expect(fallbackProviderLogo("xai").toString()).toContain(">X</text>");
    expect(() => fallbackProviderLogo("../etc")).toThrow("Invalid provider id");
  });
  it("maps provider ids to their models.dev logo slugs", async () => {
    expect(providerLogoUrl("anthropic")).toBe("/model-catalog/logos/anthropic.svg");
    expect(providerLogoUrl("openai-codex")).toBe("/model-catalog/logos/openai.svg");
    expect((await cache.get("openai-codex")).toString()).toBe(image);
    expect(fetcher.mock.calls[0]?.[0]).toBe("https://models.dev/logos/openai.svg");
  });
  it.each(["../openai", "openai.png", "", "x".repeat(65), "open ai", ".openai"])("rejects unsafe provider keys: %s", async (provider) => {
    expect(isProviderSlug(provider)).toBe(false);
    expect(providerLogoUrl(provider)).toBeNull();
    await expect(cache.get(provider)).rejects.toThrow("Invalid");
    expect(fetcher).not.toHaveBeenCalled();
  });
});