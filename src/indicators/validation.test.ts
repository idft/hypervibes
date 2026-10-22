import { describe, expect, it } from "vitest";
import { parseIndicatorId, resolveInputs, validateInputMigration, validateInputs, validateSource } from "./validation.js";
import { indicatorLimits, type IndicatorInput } from "./contracts.js";
import { adaptCandles, candleWindow } from "./candle-window.js";
import { validateNormalizedIndicatorResult } from "./result-schema.js";

const schema: IndicatorInput[] = [{ key: "period", variable: "period", runtimeId: "in_0", title: "Same title", type: "int", default: 14, min: 2, max: 500 }, { key: "source", variable: "source", runtimeId: "in_1", title: "Same title", type: "source", default: "close", options: ["open", "close"] }];
describe("indicator IDs, source and inputs", () => {
  it("separates static builtin identities from scoped UUID identities", () => {
    expect(parseIndicatorId("builtin:ema")).toEqual({ kind: "builtin", name: "ema" });
    expect(parseIndicatorId("custom:12345678-1234-4234-8234-123456789abc")).toEqual({ kind: "custom", uuid: "12345678-1234-4234-8234-123456789abc" });
    for (const id of ["ema", "builtin:nope", "builtin:constructor", "custom:ema", "12345678-1234-4234-8234-123456789abc"]) expect(() => parseIndicatorId(id)).toThrow();
  });
  it("rejects dangerous keys and nonfinite/nonscalar input values", () => {
    for (const value of [null, [], { period: Infinity }, { period: NaN }, { period: {} }, { period: [1] }, JSON.parse('{"__proto__":1}'), { constructor: "bad" }, { prototype: "bad" }]) expect(() => validateInputs(value)).toThrow();
    expect(validateInputs({ zero: 0, flag: false, str: "" })).toEqual({ zero: 0, flag: false, str: "" });
    expect(() => validateInputs(Object.fromEntries(Array.from({ length: 33 }, (_, i) => [String(i), i])))).toThrow("Too many");
  });
  it("resolves exact public keys, never titles, without mutating defaults", () => {
    expect(resolveInputs(schema, { period: 20 })).toEqual({ period: 20, source: "close" });
    expect(schema[0]!.default).toBe(14);
    for (const input of [{ period: 1 }, { period: 2.5 }, { period: "20" }, { "Same title": 20 }, { source: "high" }] as Record<string, string | number>[]) expect(() => resolveInputs(schema, input)).toThrow();
  });
  it("requires native indicator source; ignores forbidden tokens in strings/comments", () => {
    expect(() => validateSource('//@version=6\nindicator("request.security strategy library")\n// request.security("BTC")\nplot(close)')).not.toThrow();
    expect(() => validateSource('//@version=5\nindicator("test")\nfor i = 0 to 2\n    x = i\nplot(barstate.isrealtime ? 1 : 0)')).not.toThrow();
    for (const source of ['indicator("test")', '// @version=6\nindicator("test")', '//@version=6\nstrategy("test")', '//@version=6\nindicator("test")\nrequest . security("BTC")', '//@version=6\nindicator("test")\nimport x/y/1 as z']) {
      expect(() => validateSource(source)).toThrow();
    }
    expect(() => validateSource('//@version=6\nindicator("' + "😀".repeat(indicatorLimits.sourceBytes / 4) + '")')).toThrow("bytes");
  });
  it("does not silently rebind anonymous overrides across source versions", () => {
    expect(validateInputMigration(schema, schema, { period: 12 }, true)).toMatchObject({ period: 12 });
    const anonymous: IndicatorInput[] = [{ key: "in_0", runtimeId: "in_0", title: "Period", type: "int", default: 5 }];
    expect(() => validateInputMigration(anonymous, anonymous, { in_0: 9 }, true)).toThrow("Inputs changed");
    expect(() => validateInputMigration(schema, schema.slice(1), { period: 12 }, true)).toThrow("Inputs changed");
    expect(validateInputMigration(anonymous, anonymous, { in_0: 9 }, false)).toEqual({ in_0: 9 });
  });
  it("rejects malformed VM transports before application use", () => {
    for (const value of [null, {}, { values: [Infinity] }, JSON.parse('{"constructor":1}')]) expect(() => validateNormalizedIndicatorResult(value)).toThrow();
  });
});

describe("closed-only deterministic candle seeds", () => {
  const hour = 3600_000;
  const candle = (index: number) => ({ time: index * 3600, open: 10, high: 12, low: 9, close: 11, volume: 1 });
  it("aligns explicit starts upward and end downward, rejecting oversized requests", () => {
    expect(candleWindow(1, 4 * hour + 1, "1h", 5 * hour)).toMatchObject({ start: hour, end: 4 * hour, expected: 3, pineTimeframe: "60" });
    expect(() => candleWindow(0, 5001 * hour, "1h", 6000 * hour)).toThrow("5,000");
    expect(() => candleWindow(0, hour, "2h", 2 * hour)).toThrow("Invalid");
    expect(() => candleWindow(0, 3 * hour, "1h", 2 * hour)).toThrow("Invalid");
  });
  it("uses exclusive next-open closeTime and never includes the forming candle", () => {
    const window = candleWindow(0, 3 * hour + 1, "1h", 4 * hour);
    expect(adaptCandles([candle(0), candle(1), candle(2)], window)).toEqual({ candles: [0, 1, 2].map((i) => ({ open: 10, high: 12, low: 9, close: 11, volume: 1, openTime: i * hour, closeTime: (i + 1) * hour })), seedCoverage: "complete" });
    expect(() => adaptCandles([candle(0), candle(1), candle(2), candle(3)], window)).toThrow();
  });
  it("accepts only a leading missing prefix for chart seeds; never interior/trailing gaps", () => {
    const window = candleWindow(0, 4 * hour, "1h", 5 * hour);
    expect(adaptCandles([candle(2), candle(3)], window, true).seedCoverage).toBe("limited");
    expect(() => adaptCandles([candle(2), candle(3)], window)).toThrow();
    for (const candles of [[candle(0), candle(2), candle(3)], [candle(0), candle(1), candle(2)], [candle(2), candle(2), candle(3)]]) expect(() => adaptCandles(candles, window, true)).toThrow();
    expect(() => adaptCandles([{ ...candle(3), close: NaN }], window, true)).toThrow();
  });
});
