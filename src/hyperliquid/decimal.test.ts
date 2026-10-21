import { describe, expect, it } from "vitest";
import { addDecimals, compareDecimals, formatAmountText, formatMoneyText, moneyParts, parseDecimal } from "./decimal.js";

describe("string-decimal helpers", () => {
  it("parses only plain decimal strings", () => {
    expect(parseDecimal("123.4500")).toEqual({ negative: false, whole: 123n, fraction: "4500" });
    expect(parseDecimal("-12.5")).toEqual({ negative: true, whole: 12n, fraction: "5" });
    expect(parseDecimal("0")).toEqual({ negative: false, whole: 0n, fraction: "" });
    expect(parseDecimal("1e5")).toBeNull();
    expect(parseDecimal("12,345")).toBeNull();
    expect(parseDecimal("")).toBeNull();
    expect(parseDecimal("12.")).toBeNull();
  });

  it("adds decimals with integer arithmetic across scales", () => {
    expect(addDecimals("1.25", "2.5")).toBe("3.75");
    expect(addDecimals("0.1", "0.2")).toBe("0.3");
    expect(addDecimals("1000000000000.00000001", "0.00000001")).toBe("1000000000000.00000002");
    expect(addDecimals("-1.5", "2")).toBe("0.5");
    expect(addDecimals("0.1", "bogus")).toBeNull();
  });

  it("compares decimals without floats", () => {
    expect(compareDecimals("100000", "100000")).toBe(0);
    expect(compareDecimals("99999.999", "100000")).toBe(-1);
    expect(compareDecimals("100000.001", "100000")).toBe(1);
    expect(compareDecimals("0", "-0.0001")).toBe(1);
    expect(compareDecimals("x", "1")).toBeNull();
  });

  it("formats amounts with exact fractions, at least 2 decimals and thousands separators", () => {
    expect(formatAmountText("83880.0")).toBe("83,880.00");
    expect(formatAmountText("1234567")).toBe("1,234,567.00");
    expect(formatAmountText("0.00024")).toBe("0.00024");
    expect(formatAmountText("0.12345")).toBe("0.12345");
    expect(formatAmountText("-4.2")).toBe("(4.20)");
    expect(formatAmountText("-0.00000001")).toBe("(0.00000001)");
    expect(formatAmountText("-0.0")).toBe("(0.00)");
    expect(formatAmountText(null)).toBe("-");
    expect(formatAmountText("NaN")).toBe("-");
  });
  it("formats money like the legacy account table", () => {
    expect(formatMoneyText("1234567.8")).toBe("1,234,567.8000");
    expect(formatMoneyText("0.5")).toBe("0.5000");
    expect(formatMoneyText("12.34567")).toBe("12.3457");
    expect(formatMoneyText("-4.2")).toBe("(4.2000)");
    expect(formatMoneyText("-0.0")).toBe("0.0000");
    expect(formatMoneyText(null)).toBe("-");
    expect(moneyParts(formatMoneyText("1.25"))).toEqual({ whole: "1", decimals: "2500" });
    expect(moneyParts(null)).toEqual({ whole: "-", decimals: null });
  });
});