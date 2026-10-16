/** Spot identifiers are intentionally not resolved by the perpetual logo feature. */
export function isPerpetualLogoSymbol(symbol: string): boolean {
  return symbol.length <= 64 && /^(?:[A-Za-z0-9_-]+:)?[A-Za-z0-9_-]+$/.test(symbol);
}

export function currencyLogoUrl(symbol: string): string | null {
  return isPerpetualLogoSymbol(symbol) ? `/currency/${encodeURIComponent(symbol)}.svg` : null;
}
