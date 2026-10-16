// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";
import { AgentDashboardPage } from "./agent-dashboard-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const initial: AgentDashboard = { address: `0x${"12".repeat(20)}`, fetchedAt: 0, perpEquity: "12.5", spotUsdc: "3", positions: [], orders: [], errors: [] };
const source = () => FakeEventSource.instances[0]!;
class FakeEventSource extends EventTarget {
  static instances: FakeEventSource[] = [];
  close = vi.fn();
  onerror: (() => void) | null = null;
  constructor(readonly url: string) { super(); FakeEventSource.instances.push(this); }
  send(snapshot: AgentDashboard) { this.dispatchEvent(new MessageEvent("dashboard", { data: JSON.stringify(snapshot) })); }
}

describe("Agent dashboard", () => {
  let root: Root;
  let container: HTMLDivElement;
  beforeEach(async () => {
    FakeEventSource.instances = [];
    vi.stubGlobal("EventSource", FakeEventSource);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<AgentDashboardPage id="fixture" initial={initial} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  it("subscribes to backend snapshots and closes only the browser stream on unmount", async () => {
    const connection = source();
    expect(connection.url).toBe("/api/agents/fixture/dashboard/events");
    await act(async () => connection.send({ ...initial, perpEquity: "900" }));
    expect(container.textContent).toContain("903.00");
    await act(async () => root.unmount()); root = createRoot(container);
    expect(connection.close).toHaveBeenCalledOnce();
  });
  it("keeps last-known values on disconnect and clears errors after recovery", async () => {
    const connection = source();
    await act(async () => connection.onerror?.());
    expect(container.querySelector("[role=alert]")?.textContent).toContain("not current");
    expect(container.textContent).toContain("15.50");
    await act(async () => connection.send({ ...initial, live: { status: "reconnecting", sections: {
      perps: { updatedAt: 0, fresh: false }, spot: { updatedAt: null, fresh: false }, orders: { updatedAt: 0, fresh: false },
    } } }));
    expect(container.querySelector("[role=alert]")).toBeNull();
    expect(container.querySelector("[role=status]")?.textContent).toBe("Disconnected");
    expect(container.querySelector("[role=status] .bg-red-400")).not.toBeNull();
  });
  it("shows green when current, yellow when connected but stale, and red when the browser stream disconnects", async () => {
    const live: NonNullable<AgentDashboard["live"]> = { status: "connected", sections: {
      perps: { updatedAt: 0, fresh: true }, spot: { updatedAt: 0, fresh: true }, orders: { updatedAt: 0, fresh: true },
    } };
    await act(async () => source().send({ ...initial, live }));
    expect(container.querySelector("[role=status]")?.textContent).toBe("Connected");
    expect(container.querySelector("[role=status] .bg-emerald-400")).not.toBeNull();
    await act(async () => source().send({ ...initial, live: { ...live, sections: { ...live.sections, spot: { updatedAt: 0, fresh: false } } } }));
    expect(container.querySelector("[role=status]")?.textContent).toBe("Connected · Data stale");
    expect(container.querySelector("[role=status] .bg-amber-400")).not.toBeNull();
    await act(async () => source().onerror?.());
    expect(container.querySelector("[role=status]")?.textContent).toBe("Disconnected");
    expect(container.querySelector("[role=status] .bg-red-400")).not.toBeNull();
  });
  it("combines equity exactly, hides removed account details, and treats an incomplete total as unavailable", async () => {
    await act(async () => source().send({ ...initial, perpEquity: "0.1", spotUsdc: "0.2" }));
    const overview = container.querySelector('[aria-label="Account overview"]')!;
    expect(overview.textContent).toContain("Total equity0.3000");
    expect(overview.textContent).not.toContain(initial.address);
    expect(overview.textContent).not.toMatch(/Last updated|Read-only|Spot USDC|Perpetual equity/);
    expect(overview.querySelector("time")).toBeNull();
    await act(async () => source().send({ ...initial, spotUsdc: null }));
    expect(overview.textContent).toContain("Total equityUnavailable");
  });
  it("does not represent unavailable sections as empty", async () => {
    await act(async () => source().send({ ...initial, positions: null, orders: null }));
    expect(container.textContent).toContain("Positions unavailable");
    expect(container.textContent).toContain("Orders unavailable");
    expect(container.textContent).not.toContain("No open positions");
  });
  it("renders perpetual logos in positions and orders without resolving spot pairs", async () => {
    await act(async () => source().send({ ...initial,
      positions: [{ coin: "BTC", size: "1", entryPrice: "100", unrealizedPnl: "0", liquidationPrice: null, leverage: 1 }],
      orders: [{ id: 1, coin: "ETH", side: "Buy", size: "1", price: "100", type: "Limit" }, { id: 2, coin: "@107", side: "Buy", size: "1", price: "1", type: "Limit" }],
    }));
    expect([...container.querySelectorAll("tbody img")].map((image) => image.getAttribute("src"))).toEqual(["/currency/BTC.svg", "/currency/ETH.svg"]);
    expect(container.textContent).toContain("@107");
  });
  it("renders exact, signed unrealized P&L", async () => {
    const position = { coin: "BTC", size: "1", entryPrice: "100", unrealizedPnl: "0.1", liquidationPrice: null, leverage: 1 };
    await act(async () => source().send({ ...initial, positions: [position, { ...position, coin: "ETH", unrealizedPnl: "-0.3" }] }));
    const overview = container.querySelector('[aria-label="Account overview"]')!;
    expect(overview.querySelector(".text-red-400")!.textContent).toBe("(0.2000)");
  });
  it("does not subscribe for a chat-only agent and closes the previous stream on navigation", async () => {
    await act(async () => root.render(<AgentDashboardPage id="unlinked" initial={{ ...initial, address: null }} />));
    expect(FakeEventSource.instances).toHaveLength(1);
    expect(source().close).toHaveBeenCalledOnce();
    expect(container.textContent).toContain("No trading account");
  });
});
