/** Public exchange errors surfaced to users with their HTTP status. Browser-safe. */
export class HyperliquidError extends Error {
  constructor(message: string, readonly status = 400) { super(message); }
}