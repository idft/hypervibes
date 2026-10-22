/**
 * Minimal string-decimal helpers for display-only exchange amounts. Values
 * arriving from Hyperliquid are kept as strings and combined with integer
 * arithmetic; these are never used for signed amounts or order placement.
 */

/** Splits a plain decimal string like `-12.3400`; returns null when malformed. */
export function parseDecimal(value: string): { negative: boolean; whole: bigint; fraction: string } | null {
  if (!/^-?\d+(?:\.\d+)?$/.test(value) || value.includes("e") || value.includes("E")) return null;
  const negative = value.startsWith("-");
  const unsigned = negative ? value.slice(1) : value;
  const separator = unsigned.indexOf(".");
  const whole = separator === -1 ? unsigned : unsigned.slice(0, separator);
  const fraction = separator === -1 ? "" : unsigned.slice(separator + 1);
  return { negative, whole: BigInt(whole), fraction };
}

/** Scales two parsed decimals to a common denominator and returns the parts. */
function commonScale(
  left: { negative: boolean; whole: bigint; fraction: string },
  right: { negative: boolean; whole: bigint; fraction: string },
): { leftScaled: bigint; rightScaled: bigint; scale: number } {
  const scale = Math.max(left.fraction.length, right.fraction.length);
  const scaleUp = (part: { negative: boolean; whole: bigint; fraction: string }) =>
    BigInt(part.negative ? -1 : 1) * (part.whole * 10n ** BigInt(scale) + BigInt(part.fraction.padEnd(scale, "0") || "0"));
  return { leftScaled: scaleUp(left), rightScaled: scaleUp(right), scale };
}

/** Adds two plain decimal strings; returns null when either is malformed. */
export function addDecimals(left: string, right: string): string | null {
  const parsedLeft = parseDecimal(left);
  const parsedRight = parseDecimal(right);
  if (!parsedLeft || !parsedRight) return null;
  const { leftScaled, rightScaled, scale } = commonScale(parsedLeft, parsedRight);
  const total = leftScaled + rightScaled;
  return formatScaled(total, scale);
}

function formatScaled(scaled: bigint, scale: number): string {
  const negative = scaled < 0n;
  const absolute = negative ? -scaled : scaled;
  const denominator = 10n ** BigInt(scale);
  const whole = absolute / denominator;
  const fraction = (absolute % denominator).toString().padStart(scale, "0");
  const canonical = fraction.replace(/0+$/, "");
  const body = canonical ? `${whole}.${canonical}` : `${whole}`;
  return negative && absolute !== 0n ? `-${body}` : body;
}

/** Compares two plain decimal strings; null when either is malformed. */
export function compareDecimals(left: string, right: string): number | null {
  const parsedLeft = parseDecimal(left);
  const parsedRight = parseDecimal(right);
  if (!parsedLeft || !parsedRight) return null;
  const { leftScaled, rightScaled } = commonScale(parsedLeft, parsedRight);
  return leftScaled === rightScaled ? 0 : leftScaled < rightScaled ? -1 : 1;
}

/**
 * Formats a plain decimal string with fixed precision (4 places by default),
 * half-up rounding, thousands separators and negatives in parentheses.
 */
export function formatMoneyText(value: string | null, decimalPlaces: 2 | 4 = 4): string {
  if (!value) return "-";
  const parsed = parseDecimal(value);
  if (!parsed) return "-";
  // Keep one guard digit for exact half-up rounding at the requested precision.
  const guard = BigInt(decimalPlaces + 1);
  const scale = 10n ** guard;
  const scaled = parsed.whole * scale
    + BigInt(parsed.fraction.padEnd(Number(guard), "0").slice(0, Number(guard)) || "0");
  const rounded = (scaled + 5n) / 10n;
  const denominator = 10n ** BigInt(decimalPlaces);
  const whole = rounded / denominator;
  const fraction = (rounded % denominator).toString().padStart(decimalPlaces, "0");
  const grouped = whole.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const formatted = `${grouped}.${fraction}`;
  return parsed.negative && rounded !== 0n ? `(${formatted})` : formatted;
}

/** Formats an exchange amount for display: exact fraction preserved (padded to
 * at least 2 decimals), thousands separators on the whole part, and negatives in
 * parentheses. Never rounds; prices and sizes are exchange-exact. */
export function formatAmountText(value: string | null): string {
  if (!value) return "-";
  const parsed = parseDecimal(value);
  if (!parsed) return "-";
  const fraction = parsed.fraction.length < 2 ? parsed.fraction.padEnd(2, "0") : parsed.fraction;
  const grouped = parsed.whole.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const body = `${grouped}.${fraction}`;  return parsed.negative ? `(${body})` : body;
}

/** Splits formatted money into the whole part and optional decimals. */
export function moneyParts(value: string | null): { whole: string; decimals: string | null } {
  if (!value) return { whole: "-", decimals: null };
  const separator = value.indexOf(".");
  return {
    whole: separator === -1 ? value : value.slice(0, separator),
    decimals: separator === -1 ? null : value.slice(separator + 1),
  };
}