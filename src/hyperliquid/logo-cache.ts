import { SvgLogoCache, type SvgLogoSource } from "../cache/svg-logo-cache.js";
import { isPerpetualLogoSymbol } from "./currency-logo.js";

export function fallbackCurrencyLogo(symbol: string): Buffer {
  // Only validated symbols may reach the SVG template.
  if (!isPerpetualLogoSymbol(symbol)) throw new Error("Invalid currency symbol");
  return Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="8" fill="#18181b"/><text x="16" y="21" text-anchor="middle" font-family="sans-serif" font-size="${symbol.length <= 3 ? 11 : 8}" textLength="26" lengthAdjust="spacingAndGlyphs" fill="#fafafa">${symbol}</text></svg>`);
}

const source: SvgLogoSource = {
  directory: "hyperliquid/logos",
  validate: isPerpetualLogoSymbol,
  url: (symbol) => `https://app.hyperliquid.xyz/coins/${encodeURIComponent(symbol)}.svg`,
  fallback: fallbackCurrencyLogo,
};

/** Caches Hyperliquid perpetual coin logos; see {@link SvgLogoCache} for mechanics. */
export class CurrencyLogoCache {
  private readonly cache: SvgLogoCache;

  constructor(root: string, fetcher: typeof fetch = fetch) {
    this.cache = new SvgLogoCache(root, source, fetcher);
  }

  get(symbol: string): Promise<Buffer> {
    return this.cache.get(symbol);
  }

  close(): Promise<void> {
    return this.cache.close();
  }
}