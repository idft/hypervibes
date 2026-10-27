// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createTestQueryClient } from "../test/query.js";
const mocks = vi.hoisted(() => ({ enabled: vi.fn(), close: vi.fn(), invalidate: vi.fn().mockResolvedValue(undefined) }));
vi.mock("../web/agents.js", () => ({ setAgentEnabled: mocks.enabled, closeAgentPosition: mocks.close }));
vi.mock("../web/agent-queries.js", () => ({ agentQueryOptions: (id: string) => ({ queryKey: ["agent", id] }), agentsQueryOptions: () => ({ queryKey: ["agents"] }) }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => ({ invalidate: mocks.invalidate }) }));
import { AgentEnablementControl } from "./agent-enablement-control.js";
import { ClosePositionButton } from "./close-position-button.js";
import { AgentLifecycleSubscriber } from "./agent-lifecycle-subscriber.js";
import { AgentEventsProvider } from "./agent-events.js";

const outcome = (uncertain = false, remainingSize: string | null = "0.001") => ({
  requestId: "test", complete: false, remainingSize, remainingOrders: 0, warnings: uncertain ? ["Unknown outcome; reconcile"] : ["Partial fill"],
  cancel: { uncertain: false, results: [] }, close: { uncertain, results: [] },
});
describe("agent safety controls", () => {
  let root: Root, container: HTMLDivElement, client: QueryClient;
  beforeEach(() => {
    vi.clearAllMocks(); client = createTestQueryClient();
    vi.spyOn(window, "confirm").mockReturnValue(true);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });
  const renderStop = async (enabled: boolean) => act(async () => root.render(<QueryClientProvider client={client}><AgentEnablementControl id="agent" name="Desk" enabled={enabled} /></QueryClientProvider>));
  const renderClose = async (symbol = "BTC", agentId = "agent") => act(async () => root.render(<QueryClientProvider client={client}><ClosePositionButton agentId={agentId} symbol={symbol} /></QueryClientProvider>));
  const button = () => container.querySelector("button")!;
  const confirmClose = async () => {
    await act(async () => button().click());
    await act(async () => button().click());
  };

  it("updates agent and navbar caches from a disable in another tab", async () => {
    class Source extends EventTarget {
      static current: Source;
      close = vi.fn();
      constructor() { super(); Source.current = this; }
    }
    vi.stubGlobal("EventSource", Source);
    client.setQueryData(["agent", "agent"], { id: "agent", enabled: true });
    client.setQueryData(["agents"], [{ id: "agent", enabled: true, tradingEnabled: true }]);
    await act(async () => root.render(<QueryClientProvider client={client}><AgentEventsProvider agentId="agent"><AgentLifecycleSubscriber agentId="agent" /></AgentEventsProvider></QueryClientProvider>));
    await act(async () => Source.current.dispatchEvent(new MessageEvent("agent", { data: JSON.stringify({ id: "agent", enabled: false, state: "ready" }) })));
    expect(client.getQueryData(["agent", "agent"])).toMatchObject({ enabled: false });
    expect(client.getQueryData(["agents"])).toEqual([{ id: "agent", enabled: false, state: "ready", tradingEnabled: true }]);
  });

  it("arms inline without a browser prompt and cancels without disabling the agent", async () => {
    await renderStop(true);
    await act(async () => button().click());
    expect(window.confirm).not.toHaveBeenCalled();
    expect(mocks.enabled).not.toHaveBeenCalled();
    expect(button().getAttribute("aria-label")).toBe("Confirm stop");
    expect(container.textContent).not.toContain("Orders and positions stay open");
    const cancel = [...container.querySelectorAll("button")].find((item) => item.textContent === "Cancel")!;
    expect(cancel.parentElement).toBe(button().parentElement);
    expect(cancel.closest('[role="group"]')).not.toBeNull();
    expect(button().querySelector("button")).toBeNull();
    expect(button().classList.contains("bg-transparent")).toBe(true);
    expect(button().className).not.toContain("bg-red-");
    await act(async () => [...container.querySelectorAll("button")].find((item) => item.textContent === "Cancel")!.click());
    expect(button().getAttribute("aria-label")).toBe("Emergency stop");
    expect(mocks.enabled).not.toHaveBeenCalled();
  });
  it("resets confirmation on Escape, timeout, and an external enablement change", async () => {
    vi.useFakeTimers();
    await renderStop(true);
    await act(async () => button().click());
    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", cancelable: true })));
    expect(button().getAttribute("aria-label")).toBe("Emergency stop");
    expect(document.activeElement).toBe(button());
    await act(async () => button().click());
    await act(async () => vi.advanceTimersByTime(8_000));
    expect(button().getAttribute("aria-label")).toBe("Emergency stop");
    await act(async () => button().click());
    await renderStop(false);
    expect(button().getAttribute("aria-label")).toBe("Enable agent");
    expect(mocks.enabled).not.toHaveBeenCalled();
  });
  it("prevents duplicate stops and refreshes persisted state after a cleanup failure", async () => {
    let reject!: (error: Error) => void;
    mocks.enabled.mockReturnValue(new Promise((_resolve, fail) => { reject = fail; }));
    const invalidate = vi.spyOn(client, "invalidateQueries");
    await renderStop(true);
    await act(async () => button().click());
    expect(mocks.enabled).not.toHaveBeenCalled();
    await act(async () => { button().click(); button().click(); });
    expect(mocks.enabled).toHaveBeenCalledTimes(1);
    expect(mocks.enabled.mock.calls[0]![0]).toEqual({ data: { id: "agent", enabled: false } });
    expect(button().disabled).toBe(true);
    await act(async () => reject(new Error("Cleanup incomplete")));
    expect(container.textContent).toContain("Cleanup incomplete");
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["agents"] });
    expect(mocks.invalidate).toHaveBeenCalled();
  });
  it("enables with one click and no confirmation while disabled", async () => {
    mocks.enabled.mockResolvedValue({ enabled: true });
    await renderStop(false);
    expect(container.textContent).toContain("Agent disabled");
    expect(button().classList.contains("text-emerald-300")).toBe(true);
    expect(button().parentElement?.classList.contains("border-emerald-900/70")).toBe(true);
    expect(button().querySelector('svg[aria-hidden="true"] path')?.getAttribute("d")).toBe("m8 5 11 7-11 7V5Z");
    await act(async () => button().click());
    expect(container.textContent).not.toContain("Confirm enable");
    expect(container.textContent).not.toContain("Cancel");
    expect(window.confirm).not.toHaveBeenCalled();
    expect(mocks.enabled).toHaveBeenCalledTimes(1);
    expect(mocks.enabled.mock.calls[0]![0]).toEqual({ data: { id: "agent", enabled: true } });
  });
  it("keeps the HTTP close UUID after lost responses and uncertainty, then permits an explicit new remainder close", async () => {
    vi.stubGlobal("crypto", { getRandomValues: crypto.getRandomValues.bind(crypto) });
    mocks.close.mockRejectedValueOnce(new Error("Lost response"));
    await renderClose();
    await confirmClose();
    const first = mocks.close.mock.calls[0]![0] as { data: { id: string; symbol: string; requestId: string } };
    expect(first.data).toMatchObject({ id: "agent", symbol: "BTC" });
    expect(first.data.requestId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(Object.keys(first.data).sort()).toEqual(["id", "requestId", "symbol"]);
    mocks.close.mockResolvedValueOnce(outcome(true));
    await confirmClose();
    expect(mocks.close.mock.calls[1]![0]).toEqual(first);
    expect(container.textContent).toContain("Unknown outcome");
    mocks.close.mockResolvedValueOnce(outcome(false));
    await confirmClose();
    expect(mocks.close.mock.calls[2]![0]).toEqual(first);
    expect(container.textContent).toContain("Remaining size: 0.001");
    mocks.close.mockResolvedValueOnce(outcome(false));
    await confirmClose();
    expect(mocks.close.mock.calls[3]![0].data.requestId).not.toBe(first.data.requestId);
    expect(window.confirm).not.toHaveBeenCalled();
  });
  it("reports close preparation errors without sending an order and releases its submission guard", async () => {
    const webCrypto = crypto;
    vi.stubGlobal("crypto", { getRandomValues: () => { throw new Error("Random bytes unavailable"); } });
    await renderClose();
    await confirmClose();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Random bytes unavailable");
    expect(mocks.close).not.toHaveBeenCalled();
    expect(button().disabled).toBe(false);
    vi.stubGlobal("crypto", { getRandomValues: webCrypto.getRandomValues.bind(webCrypto) });
    mocks.close.mockResolvedValueOnce(outcome(false));
    await confirmClose();
    expect(mocks.close).toHaveBeenCalledOnce();
  });
  it("shows only the close icon until confirmation, prevents duplicates and honors cancellation", async () => {
    await renderClose();
    expect(button().classList.contains("bg-transparent")).toBe(true);
    expect(button().parentElement?.classList.contains("border")).toBe(false);
    expect(button().className).not.toContain("bg-red-");
    expect(button().textContent).toBe("");
    expect(button().getAttribute("aria-label")).toBe("Close BTC at market");
    expect(button().getAttribute("title")).toBe("Close BTC at market");
    expect(button().querySelector('svg[aria-hidden="true"] path')?.getAttribute("d")).toBe("m9.75 9.75 4.5 4.5m0-4.5-4.5 4.5M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z");
    const slot = button().parentElement!;
    const slotClass = slot.className;
    expect(slot.classList.contains("w-36")).toBe(true);
    expect(slot.parentElement?.classList.contains("w-36")).toBe(true);
    await act(async () => button().click());
    expect(button().textContent).toBe("Confirm close");
    expect(button().querySelector("svg")).not.toBeNull();
    expect(button().classList.contains("w-36")).toBe(true);
    expect(slot.className).toBe(slotClass);
    expect(container.querySelectorAll("button")).toHaveLength(1);
    expect(mocks.close).not.toHaveBeenCalled();
    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", cancelable: true })));
    expect(button().getAttribute("aria-label")).toBe("Close BTC at market");
    expect(mocks.close).not.toHaveBeenCalled();
    let resolve!: (value: ReturnType<typeof outcome>) => void;
    mocks.close.mockReturnValue(new Promise((done) => { resolve = done; }));
    await act(async () => button().click());
    await act(async () => { button().click(); button().click(); });
    expect(mocks.close).toHaveBeenCalledTimes(1); expect(button().disabled).toBe(true);
    await act(async () => resolve(outcome(false)));
    expect(window.confirm).not.toHaveBeenCalled();
  });
  it("resets close confirmation on Escape, timeout, and target changes", async () => {
    vi.useFakeTimers();
    await renderClose();
    await act(async () => button().click());
    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", cancelable: true })));
    expect(button().getAttribute("aria-label")).toBe("Close BTC at market");
    expect(document.activeElement).toBe(button());
    await act(async () => button().click());
    await act(async () => vi.advanceTimersByTime(8_000));
    expect(button().getAttribute("aria-label")).toBe("Close BTC at market");
    await act(async () => button().click());
    await renderClose("ETH");
    expect(button().getAttribute("aria-label")).toBe("Close ETH at market");
    await act(async () => button().click());
    await renderClose("ETH", "other-agent");
    expect(button().getAttribute("aria-label")).toBe("Close ETH at market");
    expect(mocks.close).not.toHaveBeenCalled();
    expect(window.confirm).not.toHaveBeenCalled();
  });
});
