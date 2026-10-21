// @vitest-environment jsdom
import { act } from "react";
import type { QueryClient } from "@tanstack/react-query";
import { createTestQueryClient, withTestQueryClient } from "../test/query.js";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";
const mocks = vi.hoisted(() => ({ sendAgentMessage: vi.fn(), runSubagent: vi.fn(), navigate: vi.fn(), saveSubagentSection: vi.fn(), invalidate: vi.fn().mockResolvedValue(undefined) }));
vi.mock("../web/agents.js", () => ({ sendAgentMessage: mocks.sendAgentMessage }));
vi.mock("../web/subagents.js", () => ({ runSubagent: mocks.runSubagent, saveSubagentSection: mocks.saveSubagentSection }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => ({ navigate: mocks.navigate, invalidate: mocks.invalidate }) }));
import { AgentDashboardPage as AgentDashboardPageComponent } from "./agent-dashboard-page.js";
let queryClient: QueryClient;
const AgentDashboardPage = withTestQueryClient(AgentDashboardPageComponent, () => queryClient);
const initial: AgentDashboard = { address: `0x${"12".repeat(20)}`, fetchedAt: 0, perpEquity: "12.5", spotUsdc: "3", positions: [], orders: [], errors: [] };
const source = () => FakeEventSource.instances.find((source) => source.url.endsWith("/dashboard/events"))!;
const workerSource = () => FakeEventSource.instances.find((source) => source.url.endsWith("/sub-agents/events"))!;
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
    queryClient = createTestQueryClient();
    FakeEventSource.instances = [];
    vi.stubGlobal("EventSource", FakeEventSource);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<AgentDashboardPage id="fixture" initial={initial} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  it("sends dashboard chat to the orchestrator, prevents duplicates and navigates only after acceptance", async () => {
    const section = container.querySelector('[aria-label="Chat"]')!;
    expect(section.previousElementSibling?.getAttribute("aria-label")).toBe("Sub-agents");
    const input = section.querySelector("textarea")!;
    const submit = () => section.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "Review my positions");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    mocks.sendAgentMessage.mockClear(); mocks.navigate.mockClear();
    let reject!: (error: Error) => void;
    mocks.sendAgentMessage.mockImplementationOnce(() => new Promise((_resolve, fail) => { reject = fail; }));
    await act(async () => { submit(); submit(); });
    expect(mocks.sendAgentMessage).toHaveBeenCalledOnce();
    expect(mocks.navigate).not.toHaveBeenCalled();
    const first = mocks.sendAgentMessage.mock.calls[0]![0];
    expect(first.data).toMatchObject({ id: "fixture", message: "Review my positions" });
    await act(async () => reject(new Error("offline")));
    expect(section.querySelector('[role="alert"]')?.textContent).toContain("Retry with the same message");
    expect(input.value).toBe("Review my positions");
    mocks.sendAgentMessage.mockRejectedValueOnce(new Error("Agent unavailable"));
    await act(async () => { submit(); });
    expect(section.textContent).toContain("Agent unavailable");
    expect(mocks.navigate).not.toHaveBeenCalled();
    mocks.sendAgentMessage.mockResolvedValueOnce({ submissionId: 1 });
    await act(async () => { submit(); });
    expect(mocks.sendAgentMessage.mock.calls[2]![0]).toEqual(first);
    expect(mocks.navigate).toHaveBeenCalledWith({ to: "/agents/$agentId/chat", params: { agentId: "fixture" } });
  });
  it("shows a sub-agent empty state and creation link without the Performance section", () => {
    expect(container.textContent).toContain("No sub-agents yet.");
    const creationLink = container.querySelector('a[href="/agents/fixture/sub-agents/new"]');
    expect(creationLink?.textContent).toContain("Create a new sub-agent");
    expect(creationLink?.closest("p")?.textContent).toContain("No sub-agents yet.");
    expect([...container.querySelectorAll("h3")].some((heading) => heading.textContent === "Performance")).toBe(false);
  });
  it("links worker headers and run results to their detail pages with relative timestamps", async () => {
    await act(async () => root.render(<AgentDashboardPage id="fixture" initial={initial} subagents={[
      { id: "worker:one", name: "Research", enabled: true, schedule: "", nextRunAt: null, runs: [{ id: "run:one", status: "completed", createdAt: Date.now() - 120000, result: "Market conditions look favorable." }, { id: "older", status: "failed", createdAt: 1, result: "Old result hidden" }] },
      { id: "worker:two", name: "Risk", enabled: false, schedule: "", nextRunAt: null, runs: [] },
    ]} />));
    const section = container.querySelector('[aria-label="Sub-agents"]')!;
    expect(section.querySelector('a[href="/agents/fixture/sub-agents/new"]')).toBeNull();
    expect(section.querySelector('h4 a')?.getAttribute("href")).toBe("/agents/fixture/sub-agents/worker%3Aone");
    const run = section.querySelector('a[href$="/runs/run%3Aone"]')!;
    expect(run.textContent).toContain("Market conditions look favorable.");
    expect(run.querySelector(".truncate")).not.toBeNull();
    expect(run.querySelector("time")?.textContent).toContain("ago");
    expect(run.querySelector('[aria-label="completed"]')).not.toBeNull();
    expect(section.textContent).not.toContain("Old result hidden");
    expect(section.querySelectorAll('a[href*="/runs/"]')).toHaveLength(1);
    expect(section.textContent).toContain("No runs yet.");
    expect(section.textContent).toContain("Paused");
  });
  it("keeps the previous summary visible alongside an active run until completion", async () => {
    const previous = { id: "previous", status: "completed" as const, createdAt: 1, result: "Previous summary" };
    const worker = { id: "worker", name: "Research", enabled: true, schedule: "", nextRunAt: null };
    for (const status of ["queued", "running", "completed"] as const) {
      const current = { id: "current", status, createdAt: 2, result: status === "completed" ? "New summary" : "" };
      await act(async () => root.render(<AgentDashboardPage id="fixture" initial={initial} subagents={[{ ...worker, runs: [current, previous] }]} />));
      const section = container.querySelector('[aria-label="Sub-agents"]')!;
      expect(section.querySelector(`[aria-label="${status}"]`)).not.toBeNull();
      expect(section.textContent?.includes("Previous summary")).toBe(status !== "completed");
      expect(section.textContent?.includes("New summary")).toBe(status === "completed");
      expect(section.querySelectorAll('a[href*="/runs/"]')).toHaveLength(status === "completed" ? 1 : 2);
    }
  });
  it("opens the manual assignment dialog from an icon-only button and retries with the same request ID", async () => {
    await act(async () => root.render(<AgentDashboardPage id="fixture" initial={initial} subagents={[{ id: "worker", name: "Research", enabled: false, schedule: "", nextRunAt: null, runs: [] }]} />));
    const button = container.querySelector<HTMLButtonElement>('[aria-label="Run now: Research"]')!;
    expect(button.textContent).toBe("");
    expect(button.querySelector("svg")).not.toBeNull();
    await act(async () => button.click());
    expect(document.querySelector("dialog")?.textContent).toContain("Run manually");
    const textarea = document.querySelector("dialog textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "Review BTC");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const submit = async () => act(async () => { document.querySelector("dialog form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
    mocks.runSubagent.mockRejectedValueOnce(new Error("Network failure"));
    await submit();
    expect(document.querySelector("dialog [role=alert]")?.textContent).toContain("Retry with the same task");
    mocks.runSubagent.mockResolvedValueOnce({ id: "run" });
    await submit();
    expect(mocks.runSubagent.mock.calls[0]![0].data).toMatchObject({ agentId: "fixture", id: "worker", task: "Review BTC" });
    expect(mocks.runSubagent.mock.calls[0]![0].data.requestId).toBe(mocks.runSubagent.mock.calls[1]![0].data.requestId);
    expect(mocks.navigate).toHaveBeenCalledWith({ to: "/agents/$agentId/sub-agents/$subagentId/runs/$runId", params: { agentId: "fixture", subagentId: "worker", runId: "run" } });
    expect(document.querySelector("dialog")).toBeNull();
  });
  it("toggles enabled status and shows a live countdown only for enabled schedules", async () => {
    vi.useFakeTimers();
    const now = new Date("2026-01-01T00:00:00Z").getTime();
    vi.setSystemTime(now);
    try {
      await act(async () => root.render(<AgentDashboardPage id="fixture" initial={initial} subagents={[
        { id: "worker", name: "Research", enabled: true, schedule: "1h", nextRunAt: now + 14 * 60000, runs: [] },
        { id: "manual", name: "Manual", enabled: true, schedule: "", nextRunAt: null, runs: [] },
      ]} />));
      const section = container.querySelector('[aria-label="Sub-agents"]')!;
      expect(section.textContent).toContain("Next run: in 14 minutes");
      expect(section.querySelectorAll("time")).toHaveLength(1);
      await act(async () => vi.advanceTimersByTime(60000));
      expect(section.textContent).toContain("Next run: in 13 minutes");
      expect(mocks.invalidate).not.toHaveBeenCalled();
      mocks.saveSubagentSection.mockResolvedValueOnce({ id: "worker", enabled: false, schedule: "1h", nextRunAt: null });
      await act(async () => section.querySelector<HTMLButtonElement>('[aria-label="Pause sub-agent"]')!.click());
      expect(mocks.saveSubagentSection).toHaveBeenLastCalledWith({ data: { agentId: "fixture", id: "worker", section: "enabled", settings: { enabled: false } } });
      expect(section.textContent).not.toContain("Next run:");
      mocks.saveSubagentSection.mockRejectedValueOnce(new Error("Unable to enable"));
      await act(async () => section.querySelector<HTMLButtonElement>('[aria-label="Enable sub-agent"]')!.click());
      expect(section.querySelector('[role="alert"]')?.textContent).toBe("Unable to enable");
      mocks.saveSubagentSection.mockResolvedValueOnce({ id: "worker", enabled: true, schedule: "1h", nextRunAt: now + 61 * 60000 });
      await act(async () => section.querySelector<HTMLButtonElement>('[aria-label="Enable sub-agent"]')!.click());
      expect(section.textContent).toContain("Next run: in 1 hour");
    } finally { vi.useRealTimers(); }
  });
  it("subscribes to live activity for manual workers, recovers after disconnect, and closes on unmount", async () => {
    const connection = workerSource();
    expect(connection.url).toBe("/api/agents/fixture/sub-agents/events");
    const send = (runs: unknown[]) => act(async () => connection.dispatchEvent(new MessageEvent("subagents", { data: JSON.stringify([
      { id: "worker", name: "Manual worker", enabled: false, schedule: "", nextRunAt: null, runs },
    ]) })));
    await send([{ id: "run", status: "running", createdAt: Date.now(), result: "" }]);
    const section = container.querySelector('[aria-label="Sub-agents"]')!;
    expect(section.textContent).toContain("Manual worker");
    expect(section.querySelector('[aria-label="running"]')).not.toBeNull();
    await act(async () => connection.onerror?.());
    expect(section.querySelector('[role="alert"]')?.textContent).toContain("Reconnecting");
    await send([{ id: "run", status: "completed", createdAt: Date.now(), result: "Research finished" }]);
    expect(section.querySelector('[role="alert"]')).toBeNull();
    expect(section.textContent).toContain("Research finished");
    expect(section.querySelector('[aria-label="completed"]')).not.toBeNull();
    expect(FakeEventSource.instances).toHaveLength(2);
    await act(async () => root.unmount()); root = createRoot(container);
    expect(connection.close).toHaveBeenCalledOnce();
  });
  it("shows a load failure rather than an incorrect empty state", async () => {
    await act(async () => root.render(<AgentDashboardPage id="fixture" initial={initial} subagentsError="Please retry." />));
    expect(container.querySelector('[aria-label="Sub-agents"] [role="alert"]')?.textContent).toContain("Please retry.");
    expect(container.textContent).not.toContain("No sub-agents yet.");
  });
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
  it("shows TP/SL trigger prices with financial formatting and exact fractions", async () => {
    await act(async () => source().send({ ...initial, orders: [
      { id: 1, coin: "BTC", side: "Sell", type: "Stop Market", size: "0.00024", triggerPrice: "83880.0", price: "75492.0" },
      { id: 2, coin: "BTC", side: "Sell", type: "Take Profit Market", size: "0.00024", triggerPrice: "86420.0", price: "77778.0" },
      { id: 3, coin: "BTC", side: "Buy", type: "Limit", size: "0.00024", price: "80000.1234" },
      { id: 4, coin: "BTC", side: "Sell", type: "Stop Limit", size: "0.00024", triggerPrice: "83000.1234", price: "82900.1234" },
    ] }));
    const section = [...container.querySelectorAll("section")].find((section) => section.querySelector("h3")?.textContent === "Open orders")!;
    expect(section.querySelector("thead")?.textContent).toContain("Price / trigger");
    expect(section.textContent).not.toContain("Execution limit");
    const rows = [...section.querySelectorAll("tbody tr")];
    expect(rows.map((row) => row.querySelectorAll("td")[4]!.querySelector("span")?.textContent)).toEqual(["83,880.00", "86,420.00", "80,000.1234", "83,000.1234"]);
    expect(rows.map((row) => row.querySelectorAll("td")[3]!.querySelector("span")?.textContent)).toEqual(["0.00024", "0.00024", "0.00024", "0.00024"]);
    for (const row of rows) for (const index of [3, 4]) expect(row.querySelectorAll("td")[index]!.querySelector("span")?.className).toContain("font-mono");
  });
  it("formats position sizes, prices and P&L with separators and monospaced digits", async () => {
    await act(async () => source().send({ ...initial, positions: [
      { coin: "BTC", size: "-0.00000001", entryPrice: "100.00000001", unrealizedPnl: "-1.23456789", liquidationPrice: "1234.5", leverage: 2 },
      { coin: "ETH", size: "2", entryPrice: "2500", unrealizedPnl: "0", liquidationPrice: null, leverage: 3 },
    ] }));
    const section = [...container.querySelectorAll("section")].find((section) => section.querySelector("h3")?.textContent === "Positions")!;
    const first = section.querySelectorAll("tbody tr")[0]!;
    expect(first.querySelectorAll("td")[1]!.querySelector("span")?.textContent).toBe("(0.00000001)");
    expect(first.querySelectorAll("td")[3]!.querySelector("span")?.textContent).toBe("100.00000001");
    expect(first.querySelectorAll("td")[4]!.querySelector("span")?.textContent).toBe("(1.2346)");
    expect(first.querySelectorAll("td")[5]!.querySelector("span")?.textContent).toBe("1,234.50");
    const second = section.querySelectorAll("tbody tr")[1]!;
    expect(second.querySelectorAll("td")[1]!.querySelector("span")?.textContent).toBe("2.00");
    expect(second.querySelectorAll("td")[5]!.querySelector("span")?.textContent).toBe("-");
    for (const row of [first, second]) for (const index of [1, 3, 4, 5]) expect(row.querySelectorAll("td")[index]!.querySelector("span")?.className).toContain("font-mono");
  });
  it("renders exact, signed unrealized P&L", async () => {
    const position = { coin: "BTC", size: "1", entryPrice: "100", unrealizedPnl: "0.1", liquidationPrice: null, leverage: 1 };
    await act(async () => source().send({ ...initial, positions: [position, { ...position, coin: "ETH", unrealizedPnl: "-0.3" }] }));
    const overview = container.querySelector('[aria-label="Account overview"]')!;
    expect(overview.querySelector(".text-red-400")!.textContent).toBe("(0.2000)");
  });
  it("handles an invalid development agent without opening an account feed", async () => {
    await act(async () => root.render(<AgentDashboardPage id="unlinked" initial={{ ...initial, address: null }} />));
    expect(FakeEventSource.instances.filter((source) => source.url.endsWith("/dashboard/events"))).toHaveLength(1);
    expect(source().close).toHaveBeenCalledOnce();
    expect(container.textContent).toContain("missing its required Hyperliquid account");
  });
});
