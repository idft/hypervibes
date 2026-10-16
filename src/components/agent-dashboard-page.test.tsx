// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";

const mocks = vi.hoisted(() => ({ getAgentDashboard: vi.fn() }));
vi.mock("../web/agents.js", () => mocks);
import { AgentDashboardPage } from "./agent-dashboard-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const initial: AgentDashboard = { address: `0x${"12".repeat(20)}`, fetchedAt: 0, perpEquity: "12.5", spotUsdc: "3", positions: [], orders: [], errors: [] };

describe("Agent dashboard", () => {
  let root: Root;
  let container: HTMLDivElement;
  beforeEach(async () => {
    vi.useFakeTimers(); vi.resetAllMocks();
    vi.spyOn(document, "hidden", "get").mockReturnValue(false);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    mocks.getAgentDashboard.mockResolvedValue({ ok: true, data: initial });
    await act(async () => root.render(<AgentDashboardPage id="fixture" initial={initial} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.restoreAllMocks(); });
  it("refreshes only while the dashboard is mounted and visible", async () => {
    expect(mocks.getAgentDashboard).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTimeAsync(15_000));
    expect(mocks.getAgentDashboard).toHaveBeenCalledOnce();
    vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    await act(async () => vi.advanceTimersByTimeAsync(30_000));
    expect(mocks.getAgentDashboard).toHaveBeenCalledOnce();
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => vi.advanceTimersByTimeAsync(30_000));
    expect(mocks.getAgentDashboard).toHaveBeenCalledOnce();
  });
  it("does not overlap refreshes or reschedule after unmount during a request", async () => {
    let resolve!: (value: unknown) => void;
    mocks.getAgentDashboard.mockImplementation(() => new Promise((done) => { resolve = done; }));
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(mocks.getAgentDashboard).toHaveBeenCalledOnce();
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(mocks.getAgentDashboard).toHaveBeenCalledOnce();
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => resolve({ ok: true, data: initial }));
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(mocks.getAgentDashboard).toHaveBeenCalledOnce();
  });
  it("labels old data on refresh failure and does not represent unavailable sections as empty", async () => {
    mocks.getAgentDashboard.mockRejectedValue(new Error("offline"));
    await act(async () => vi.advanceTimersByTimeAsync(15_000));
    expect(container.querySelector("[role=alert]")?.textContent).toContain("not current");
    await act(async () => root.render(<AgentDashboardPage key="unavailable" id="fixture" initial={{ ...initial, positions: null, orders: null, errors: ["Exchange unavailable"] }} />));
    expect(container.textContent).toContain("Positions unavailable");
    expect(container.textContent).toContain("Orders unavailable");
    expect(container.textContent).not.toContain("No open positions");
  });
  it("renders legacy-style overview statistics with exact, signed unrealized P&L", async () => {
    const position = { coin: "BTC", size: "1", entryPrice: "100", unrealizedPnl: "0.1", liquidationPrice: null, leverage: 1 };
    await act(async () => root.render(<AgentDashboardPage key="stats" id="fixture" initial={{ ...initial, positions: [position, { ...position, coin: "ETH", unrealizedPnl: "-0.3" }] }} />));
    const overview = container.querySelector('[aria-label="Account overview"]')!;
    expect(overview.textContent).toContain("Perpetual equity");
    expect(overview.textContent).toContain("Spot USDC");
    expect(overview.querySelector(".text-red-400")!.textContent).toBe("(0.2000)");
    expect(overview.textContent).toContain("History not available yet");
  });
  it("does not poll for a chat-only agent", async () => {
    await act(async () => root.render(<AgentDashboardPage key="unlinked" id="fixture" initial={{ ...initial, address: null }} />));
    await act(async () => vi.advanceTimersByTimeAsync(60_000));
    expect(mocks.getAgentDashboard).not.toHaveBeenCalled();
    expect(container.textContent).toContain("No trading account");
  });
});
