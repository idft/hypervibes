import { HyperliquidError } from "../errors.js";

/** Signing-path decimal arithmetic. No conversion of financial values to Number. */
export function decimal(value: string): { units: bigint; scale: number; text: string } {
  if (typeof value !== "string" || value.length > 80 || value.trim() !== value || !/^(0|[1-9]\d*)(\.\d+)?$/.test(value)) throw new HyperliquidError("Use a positive plain decimal string.");
  const [whole = "", fraction = ""] = value.split(".");
  const trimmed = fraction.replace(/0+$/, "");
  const units = BigInt(whole + trimmed);
  if (units <= 0n) throw new HyperliquidError("Order prices and sizes must be greater than zero.");
  return { units, scale: trimmed.length, text: trimmed ? `${whole}.${trimmed}` : whole };
}
export function compare(left: string, right: string): number {
  const a = decimal(left), b = decimal(right);
  const x = a.units * 10n ** BigInt(b.scale), y = b.units * 10n ** BigInt(a.scale);
  return x === y ? 0 : x < y ? -1 : 1;
}
export function precisionRules(sizeDecimals: number) {
  if (!Number.isInteger(sizeDecimals) || sizeDecimals < 0 || sizeDecimals > 6) throw new HyperliquidError("Exchange size precision unavailable or unsupported", 502);
  return { sizeDecimals, sizeStep: format(1n, sizeDecimals), priceMaxDecimals: 6 - sizeDecimals, priceMaxSignificantFigures: 5, integerPricesAllowed: true, minOpeningNotional: "10" };
}
export function size(value: string, decimals: number): string {
  const parsed = decimal(value);
  if (parsed.scale > decimals) throw new HyperliquidError(`Size exceeds exchange precision: received ${JSON.stringify(value)}; at most ${decimals} decimal places (size step ${format(1n, decimals)}). Choose a valid base-asset size within the requested budget; do not automatically increase it.`);
  return parsed.text;
}
export function price(value: string, sizeDecimals: number): string {
  const parsed = decimal(value);
  // Integers are always permitted. Fractional perp prices have <=5 significant
  // figures and <=(6 - szDecimals) decimal places, per Hyperliquid's tick rules.
  if (parsed.scale && (parsed.scale > 6 - sizeDecimals || parsed.units.toString().length > 5)) {
    throw new HyperliquidError(`Price exceeds exchange tick/significant-figure precision: received ${JSON.stringify(value)}; fractional perpetual prices allow at most ${6 - sizeDecimals} decimal places AND 5 significant figures. Positive integer prices are exempt from both precision limits. Choose an explicit valid price consistent with the requested levels.`);
  }
  return parsed.text;
}
export function format(units: bigint, scale: number): string {
  const padded = units.toString().padStart(scale + 1, "0");
  return scale ? `${padded.slice(0, -scale)}.${padded.slice(-scale)}` : padded;
}
/** Exact notional comparison; never round financial authority through Number. */
export function notionalWithin(qty: string, px: string, cap: string): boolean {
  const a = decimal(qty), b = decimal(px), c = decimal(cap);
  return a.units * b.units * 10n ** BigInt(c.scale) <= c.units * 10n ** BigInt(a.scale + b.scale);
}
export function notionalAtLeast(qty: string, px: string, minimum: string): boolean {
  const a = decimal(qty), b = decimal(px), c = decimal(minimum);
  return a.units * b.units * 10n ** BigInt(c.scale) >= c.units * 10n ** BigInt(a.scale + b.scale);
}
/** Exact product, with no rounding below a risk cap. */
export function notional(qty: string, px: string): string {
  const a = decimal(qty), b = decimal(px);
  return format(a.units * b.units, a.scale + b.scale);
}
/** Exact sum of plain decimal strings. */
export function sumDecimals(values: string[]): string {
  const parsed = values.map(decimal);
  const scale = Math.max(0, ...parsed.map((value) => value.scale));
  const total = parsed.reduce((sum, value) => sum + value.units * 10n ** BigInt(scale - value.scale), 0n);
  return format(total, scale);
}
/** Exact position size/sign; exchange positions may contain zero or a minus sign. */
export function positionSize(value: string): { side: "long" | "short" | "flat"; size: string } {
  if (typeof value !== "string" || value.trim() !== value || !/^-?(0|[1-9][0-9]*)(\.[0-9]+)?$/.test(value)) throw new HyperliquidError("Invalid exchange position size");
  if (!/[1-9]/.test(value)) return { side: "flat", size: "0" };
  return { side: value.startsWith("-") ? "short" : "long", size: decimal(value.replace(/^-/, "")).text };
}
/** Guarded IOC price: 0.5% for entries, 10% for market triggers, rounded inward. */
export function guardPrice(reference: string, buy: boolean, sizeDecimals: number, bps = 50): string {
  const parsed = decimal(reference);
  const numerator = parsed.units * BigInt(10_000 + (buy ? bps : -bps));
  const denominator = 10n ** BigInt(parsed.scale) * 10_000n;
  const whole = numerator / denominator;
  let magnitude = whole ? whole.toString().length - 1 : -1;
  if (!whole) {
    let scaled = numerator * 10n;
    while (scaled < denominator) { magnitude--; scaled *= 10n; }
  }
  const scale = Math.max(0, Math.min(6 - sizeDecimals, 4 - magnitude));
  const scaledNumerator = numerator * 10n ** BigInt(scale);
  const rounded = (scaledNumerator + (buy ? 0n : denominator - 1n)) / denominator;
  return price(format(rounded, scale), sizeDecimals);
}
