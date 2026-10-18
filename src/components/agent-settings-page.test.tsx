// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentChat } from "../agents/agent-service.js";
const mocks = vi.hoisted(() => ({ save: vi.fn(), success: vi.fn() }));
vi.mock("../web/risk.js", () => ({ saveRiskSettings: mocks.save }));
vi.mock("sonner", () => ({ toast: { success: mocks.success } }));
import { AgentSettingsPage } from "./agent-settings-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const initial: AgentChat = {
  agent: { id: "agent-fixture", name: "Research", userId: "owner", state: "ready", conversationId: 1, initialProvider: null, initialModelId: null, error: null, tradingAccountAddress: null, creationRequestId: "fixture", createdAt: 0, updatedAt: 0 },
  model: { provider: "current", modelId: "old" }, configured: true, busy: false, queuedMessages: [], messages: [], partial: "",
  usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 }, history: false, compacting: false,
};
describe("Agent settings", () => {
  let root: Root; let container: HTMLDivElement;
  const eventSource = vi.fn();
  beforeEach(async () => {
    vi.resetAllMocks();
    mocks.save.mockResolvedValue({ ok: true, data: {} });
    vi.stubGlobal("EventSource", eventSource);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<AgentSettingsPage initial={initial} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  async function change(label: string, value: string) {
    await act(async () => {
      const element = container.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(element, value);
      element.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  async function save() { await act(async () => { [...container.querySelectorAll("button")].find((button) => button.textContent === "Save risk settings")!.click(); }); }
  it("saves risk settings on the Settings page with explicit all-allowed and uncapped values", async () => {
    expect(container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(false);
    await change("Allowed instruments", "BTC, ETH");
    await change("Max order notional", "100.01");
    await change("Max total notional", "500");
    await act(async () => container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
    await save();
    expect(mocks.save).toHaveBeenLastCalledWith({ data: { agentId: initial.agent.id, settings: { tradingEnabled: true, allowedInstruments: ["BTC", "ETH"], maxOrderNotional: "100.01", maxTotalNotional: "500" } } });
    await change("Allowed instruments", ""); await change("Max order notional", ""); await change("Max total notional", "");
    await save();
    expect(mocks.save).toHaveBeenLastCalledWith({ data: { agentId: initial.agent.id, settings: { tradingEnabled: true, allowedInstruments: null, maxOrderNotional: null, maxTotalNotional: null } } });
  });
  it("preserves the draft when saving fails", async () => {
    mocks.save.mockResolvedValue({ ok: false, error: "Unable to save" });
    await change("Max total notional", "123.45");
    await save();
    expect(container.querySelector('[role="alert"]')!.textContent).toContain("Unable to save");
    expect(container.querySelector<HTMLInputElement>('[aria-label="Max total notional"]')!.value).toBe("123.45");
  });
  it("keeps application settings and links to the protected orchestrator instead of selecting models here", () => {
    expect(container.textContent).toContain("Execution backend"); expect(container.textContent).toContain("Telegram");
    expect(container.querySelector("select")).toBeNull();
    expect(container.querySelector("a")?.getAttribute("href")).toBe("/agents/agent-fixture/sub-agents/main%3Aagent-fixture");
    expect(eventSource).not.toHaveBeenCalled();
  });
});
