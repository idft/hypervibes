// @vitest-environment jsdom
import { act } from "react";
import { hydrateRoot } from "react-dom/client";
import { renderToStaticMarkup, renderToString } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { BalanceSparkline } from "./balance-sparkline.js";


const render = (balances: string[]) => renderToStaticMarkup(<BalanceSparkline label="24h" points={balances.map((balance, time) => ({ time, balance }))} />);
describe("BalanceSparkline", () => {
  it("renders accessible positive, negative and flat charts", () => {
    expect(render(["1", "2"])).toContain("+1.0000 USDC");
    expect(render(["1", "2"])).toContain('aria-label="24h realized account balance"');
    expect(render(["2", "1"])).toContain("text-red-400");
    expect(render(["2", "1"])).toContain("(1.0000) USDC");
    expect(render(["2", "2"])).toContain('points="0.00,24.00 240.00,24.00"');
  });
  it("hydrates the server-rendered SVG title without a mismatch", async () => {
    const chart = <BalanceSparkline label="24h" points={[{ time: 0, balance: "1" }, { time: 1, balance: "2" }]} />;
    const container = document.createElement("div");
    container.innerHTML = renderToString(chart);
    document.body.append(container);
    const errors: unknown[] = [];
    let root: ReturnType<typeof hydrateRoot> | undefined;
    try {
      await act(async () => { root = hydrateRoot(container, chart, { onRecoverableError: (error) => { errors.push(error); } }); });
      expect(errors).toEqual([]);
      expect(container.querySelector("svg title")?.textContent).toBe("24h realized account balance (USDC)");
    } finally {
      await act(async () => { root?.unmount(); });
      container.remove();
    }
  });
  it("normalizes exact differences before converting to drawing coordinates", () => {
    expect(render(["1000000000000000000.00000001", "1000000000000000000.00000002"])).toContain('points="0.00,44.00 240.00,4.00"');
  });
  it("shows an honest empty state for missing or insufficient history", () => {
    expect(renderToStaticMarkup(<BalanceSparkline label="30d" />)).toContain("Balance history unavailable");
    expect(render(["1"])).not.toContain("<svg");
  });
});
