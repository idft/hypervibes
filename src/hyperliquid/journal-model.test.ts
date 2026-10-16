import { describe, expect, it } from "vitest";
import { amount } from "./journal-decimal.js";
import { buildTrades, fillEvent, ledgerEvent, type JournalEvent } from "./journal-model.js";

export function fill(id: number, side: string, size: string, start: string | null, fee = "0.1", pnl: string | null = "0", time = id): JournalEvent {
  return { id: String(id), kind: "fill", time, symbol: "BTC", type: `fill ${side}`, fee, pnl,
    delta: pnl === null ? null : amount(pnl).sub(amount(fee)).toString(), raw: {},
    fill: { hash: String(id), tid: String(id), side, size, price: "100", start, rawFee: fee, feeToken: "USDC" } };
}

describe("journal exact accounting and legacy trade-cycle fixtures", () => {
  it("keeps large amounts, products and tiny deltas exact; rounds division half-even", () => {
    expect(amount("9007199254740993.000000000000000001").add(amount("0.000000000000000001")).toString()).toBe("9007199254740993.000000000000000002");
    expect(amount("0.0000000001").mul(amount("0.0000000001")).toString()).toBe("0.00000000000000000001");
    expect(amount("1").div(amount("3")).toString()).toBe("0.3333333333333333333333333333");
    expect(amount("0.00000000000000000000000000005").div(amount("1")).toString()).toBe("0");
    expect(amount("0.00000000000000000000000000015").div(amount("1")).toString()).toBe("0.0000000000000000000000000002");
    expect(amount("-0.000").toString()).toBe("0");
    for (const value of ["NaN", "1e-8", "Infinity", "", "+1", " 1"]) expect(() => amount(value)).toThrow();
  });

  it("ports the Rust flip fixture, preserving fees, P&L and stable IDs", () => {
    const events = [fill(1, "B", "2", "0", "2"), fill(2, "A", "3", "2", "3", "10")];
    const trades = buildTrades("account", events);
    expect(trades).toHaveLength(2);
    expect(trades[0]).toMatchObject({ status: "closed", grossPnl: "10", fees: "4", entrySize: "2", exitSize: "2", net: "6" });
    expect(trades[1]).toMatchObject({ direction: "short", status: "open", fees: "1", remainingSize: "1" });
    expect(buildTrades("account", [...events].reverse()).map((t) => t.id)).toEqual(trades.map((t) => t.id));
    const thirds = buildTrades("account", [fill(1, "B", "1", "0", "0"), fill(2, "A", "3", "1", "1")]);
    expect(amount(thirds[0]!.fees).add(amount(thirds[1]!.fees)).toString()).toBe("1");
  });
  it("groups scale-ins and partial exits but separates reentries (Rust fixture)", () => {
    const trades = buildTrades("a", [fill(1, "buy", "1", "0"), fill(2, "B", "1", "1"), fill(3, "sell", "0.5", "2", "0.05", "2"), fill(4, "A", "1.5", "1.5", "0.15", "5"), fill(5, "sell", "1", "0"), fill(6, "buy", "1", "-1", "0.1", "3")]);
    expect(trades).toHaveLength(2);
    expect(trades[0]).toMatchObject({ status: "closed", entrySize: "2", exitSize: "2", grossPnl: "7" });
    expect(trades[1]).toMatchObject({ status: "closed", direction: "short", grossPnl: "3" });
  });
  it("orders timestamp ties by position chain rather than exchange IDs (Rust fixture)", () => {
    const trades = buildTrades("a", [fill(1, "A", "0.0002", "0", "0.01", "0", 1), fill(2, "A", "0.0002", "-0.0002", "0.01", "0", 2), fill(3, "B", "0.0002", "-0.0002", "0.01", "-0.0644", 3), fill(4, "B", "0.0002", "-0.0004", "0.01", "-0.0644", 3)]);
    expect(trades).toHaveLength(1);
    expect(trades[0]).toMatchObject({ status: "closed", grossPnl: "-0.1288", fees: "0.04", remainingSize: "0", coverageReason: null });
    expect(trades[0]!.parts.map((p) => p.eventId)).toEqual(["1", "2", "4", "3"]);
  });
  it("marks gaps, missing start/P&L, unknown sides and non-USDC fees incomplete", () => {
    expect(buildTrades("a", [fill(1, "A", "1", "2")])[0]).toMatchObject({ status: "incomplete", entrySize: "0", funding: null, net: null });
    expect(buildTrades("a", [fill(1, "?", "1", "0")])[0]).toMatchObject({ status: "incomplete", direction: "unknown" });
    expect(buildTrades("a", [fill(1, "B", "1", null)])[0]?.status).toBe("incomplete");
    expect(buildTrades("a", [fill(1, "B", "1", "0"), fill(2, "A", "1", "1", "1", null)])[0]?.status).toBe("incomplete");
    const event = fill(1, "B", "1", "0"); event.fee = null;
    expect(buildTrades("a", [event])[0]).toMatchObject({ feeCoverage: false, net: null });
    expect(buildTrades("a", [fill(1, "B", "1", "0"), fill(2, "A", "1", "3")]).every((t) => t.status === "incomplete")).toBe(true);
  });
  it("allocates signed funding over the half-open holding interval", () => {
    const funding = (id: string, time: number, delta: string): JournalEvent => ({ id, time, delta, kind: "funding", symbol: "BTC", type: "funding", pnl: null, fee: null, fill: null, raw: {} });
    const trades = buildTrades("a", [fill(1, "B", "1", "0", "0.1", "0", 10), fill(2, "A", "1", "1", "0.1", "3", 20), funding("f1", 9, "100"), funding("f2", 10, "-0.25"), funding("f3", 20, "100")]);
    expect(trades[0]).toMatchObject({ funding: "-0.25", net: "2.55" });
  });
  it("keeps spot out of perpetual cycles and preserves unknown ledger cash impact", () => {
    const event = fillEvent({ coin: "@107", side: "A", sz: "1", px: "100", time: 1, hash: "0x00", tid: 1, oid: 1, twapId: null, crossed: false, dir: "Sell", startPosition: "1", fee: "0.1", feeToken: "USDC", closedPnl: "10" });
    expect(event.delta).toBeNull(); expect(buildTrades("a", [event])).toEqual([]);
    const withdrawal = ledgerEvent("a", { hash: "0x00", time: 2, delta: { type: "withdraw", usdc: "10", fee: "1", nonce: 1 } });
    expect(withdrawal.delta).toBe("-10");
    expect(ledgerEvent("a", { hash: "0x01", time: 3, delta: { type: "accountClassTransfer", usdc: "10", toPerp: true } }).delta).toBeNull();
  });
});
