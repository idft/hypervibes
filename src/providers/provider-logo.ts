import { SvgLogoCache, type SvgLogoSource } from "../cache/svg-logo-cache.js";
import { isProviderSlug, providerLogoSlug } from "./provider-logo-url.js";

export { isProviderSlug, providerLogoUrl } from "./provider-logo-url.js";

export function fallbackProviderLogo(provider: string): Buffer {
  // Only validated provider ids may reach the SVG template.
  if (!isProviderSlug(provider)) throw new Error("Invalid provider id");
  const initial = provider[0]!.toUpperCase();
  return Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" role="img" aria-label="${provider}"><rect width="32" height="32" rx="8" fill="#18181b"/><text x="16" y="21" text-anchor="middle" font-family="ui-sans-serif,system-ui" font-size="14" fill="#fafafa">${initial}</text></svg>`);
}

const source: SvgLogoSource = {
  directory: "model-catalog/logos",
  validate: isProviderSlug,
  url: (slug) => `https://models.dev/logos/${slug}.svg`,
  fallback: fallbackProviderLogo,
};

/** Caches models.dev provider logos; see {@link SvgLogoCache} for mechanics. */
export class ProviderLogoCache {
  private readonly cache: SvgLogoCache;

  constructor(root: string, fetcher: typeof fetch = fetch) {
    this.cache = new SvgLogoCache(root, source, fetcher);
  }

  get(provider: string): Promise<Buffer> {
    // Aliases resolve before caching so each models.dev slug owns one disk entry.
    const slug = providerLogoSlug(provider);
    return slug ? this.cache.get(slug) : Promise.reject(new Error("Invalid logo key"));
  }

  close(): Promise<void> {
    return this.cache.close();
  }
}