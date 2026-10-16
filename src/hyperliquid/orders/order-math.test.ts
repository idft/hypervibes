import { describe, expect, it } from "vitest";
import { compare, decimal, guardPrice, price, size } from "./order-math.js";
import { assertStringAmounts } from "./order-prepare.js";

describe("exact order signing arithmetic", () => {
  it("validates positive decimal strings without float/exponent conversion", () => {
    expect(decimal("9007199254740993.123400").text).toBe("9007199254740993.1234");
    expect(compare("9007199254740993.01", "9007199254740993.00")).toBe(1);
    for (const value of ["0", "-1", "1e3", "NaN", "01", ".1", "1.", "9".repeat(81)]) expect(() => decimal(value)).toThrow();
    expect(() => assertStringAmounts({ orders: [{ size: 0.01 }] })).toThrow("decimal string");
    expect(() => assertStringAmounts({ orders: [{ take_profits: [{ trigger_price: 100 }] }] })).toThrow();
    expect(size("0.12345000", 5)).toBe("0.12345");
    expect(() => size("0.123456", 5)).toThrow("precision");
  });
  it("enforces significant figures and exchange decimal precision while permitting integers", () => {
    expect(price("123456789", 5)).toBe("123456789");
    expect(price("1234.5", 1)).toBe("1234.5");
    expect(price("0.012345", 0)).toBe("0.012345");
    expect(() => price("1234.56", 0)).toThrow();
    expect(() => price("1.234", 5)).toThrow();
  });
  it("builds inward-rounded guarded IOC and market trigger prices exactly", () => {
    expect(guardPrice("100000", true, 5)).toBe("100500");
    expect(guardPrice("100000", false, 5)).toBe("99500");
    expect(guardPrice("1234.5", true, 3)).toBe("1240.6");
    expect(guardPrice("1234.5", false, 3)).toBe("1228.4");
    expect(guardPrice("100", false, 3, 1000)).toBe("90");
    expect(guardPrice("9007199254740993", true, 0)).toBe("9052235251014697");
    expect(() => guardPrice("0.00000001", true, 5)).toThrow();
  });
});
