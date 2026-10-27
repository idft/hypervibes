// @vitest-environment jsdom
import { act } from "react";
import type { QueryClient } from "@tanstack/react-query";
import { createTestQueryClient, withTestQueryClient } from "../test/query.js";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentChat } from "../agents/agent-service.js";
import type { RiskSettings } from "../hyperliquid/orders/risk-settings.js";
const mocks = vi.hoisted(() => ({ save: vi.fn(), setEnabled: vi.fn(), success: vi.fn(), invalidate: vi.fn(), instruments: vi.fn(), remove: vi.fn(), navigate: vi.fn(), addToken: vi.fn(), startLink: vi.fn(), disconnect: vi.fn(), status: vi.fn() }));
vi.mock("../web/charts.js", () => ({ getChartInstruments: mocks.instruments }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => ({ invalidate: mocks.invalidate, navigate: mocks.navigate }) }));
vi.mock("../web/agents.js", () => ({ deleteAgent: mocks.remove, setAgentEnabled: mocks.setEnabled }));
vi.mock("../web/risk.js", () => ({ saveRiskSettings: mocks.save }));
vi.mock("../web/gateway.js", () => ({ addGatewayToken: mocks.addToken, startGatewayLink: mocks.startLink, disconnectGateway: mocks.disconnect, getGatewayStatus: mocks.status }));
vi.mock("sonner", () => ({ toast: { success: mocks.success } }));
import { AgentSettingsPage as AgentSettingsPageComponent } from "./agent-settings-page.js";
let queryClient: QueryClient;
const AgentSettingsPage = withTestQueryClient(AgentSettingsPageComponent, () => queryClient);
const initial: AgentChat = {
  agent: { id: "agent-fixture", name: "Research", userId: "owner", enabled: true, state: "ready", conversationId: 1, initialProvider: null, initialModelId: null, error: null, tradingAccountAddress: null, creationRequestId: "fixture", createdAt: 0, updatedAt: 0 },
  model: { provider: "current", modelId: "old" }, configured: true, busy: false, queuedMessages: [], messages: [], partial: "",
  usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 }, history: false, compacting: false,
};
describe("Agent settings", () => {
  let root: Root; let container: HTMLDivElement;
  const eventSource = vi.fn();
  beforeEach(async () => {
    vi.resetAllMocks();
    queryClient = createTestQueryClient({ staleTime: Infinity });
    queryClient.setQueryData(["risk", initial.agent.id], { tradingEnabled: false, allowedInstruments: null, maxOrderNotional: null, maxTotalNotional: null });
    queryClient.setQueryData(["notification-settings", initial.agent.id], { enabled: true, submissions: true, fills: true });
    queryClient.setQueryData(["gateway", initial.agent.id], { hasToken: false, connected: false, botUsername: null, chatId: null, chatUsername: null });
    mocks.instruments.mockResolvedValue(["BTC", "ETH", "SOL"]);
    mocks.save.mockImplementation(async ({ data }: { data: { settings: RiskSettings } }) => data.settings);
    vi.stubGlobal("EventSource", eventSource);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<AgentSettingsPage initial={initial} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  it("shows enablement without duplicate lifecycle readiness text and can disable the agent", async () => {
    expect(container.querySelector('[aria-label="Agent execution"]')).toBeNull();
    expect(container.textContent).not.toContain("Lifecycle status describes initialization");
    const lifecycle = [...container.querySelectorAll("dt")].find((term) => term.textContent === "Lifecycle status")!.nextElementSibling!;
    expect(lifecycle.textContent).not.toContain("ready");
    expect(lifecycle.querySelector('[role="status"]')?.textContent).toBe("Agent enabled");
    mocks.setEnabled.mockResolvedValue({ ...initial.agent, enabled: false });
    await act(async () => lifecycle.querySelector<HTMLButtonElement>('[aria-label="Disable agent"]')!.click());
    expect(mocks.setEnabled).not.toHaveBeenCalled();
    expect(lifecycle.querySelector('[aria-label="Confirm stop"]')!.parentElement?.textContent).toBe("Confirm stopCancel");
    await act(async () => lifecycle.querySelector<HTMLButtonElement>('[aria-label="Confirm stop"]')!.click());
    expect(mocks.setEnabled.mock.calls[0]![0]).toEqual({ data: { id: initial.agent.id, enabled: false } });
  });
  it("shows ready-but-disabled accurately and offers re-enablement", async () => {
    await act(async () => root.render(<AgentSettingsPage initial={{ ...initial, agent: { ...initial.agent, enabled: false } }} />));
    const lifecycle = [...container.querySelectorAll("dt")].find((term) => term.textContent === "Lifecycle status")!.nextElementSibling!;
    expect(lifecycle.textContent).not.toContain("ready");
    expect(lifecycle.querySelector('[role="status"]')?.textContent).toBe("Agent disabled");
    mocks.setEnabled.mockResolvedValue(initial.agent);
    await act(async () => lifecycle.querySelector<HTMLButtonElement>('[aria-label="Enable agent"]')!.click());
    expect(lifecycle.querySelector('[aria-label="Confirm enable"]')).toBeNull();
    expect(mocks.setEnabled).toHaveBeenCalledTimes(1);
    expect(mocks.setEnabled.mock.calls[0]![0]).toEqual({ data: { id: initial.agent.id, enabled: true } });
  });
  async function change(label: string, value: string) {
    await act(async () => {
      const element = container.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(element, value);
      element.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  /** Like change(), but for inputs rendered into the dialog portal in document.body. */
  async function dialogChange(label: string, value: string) {
    await act(async () => {
      const element = document.querySelector<HTMLInputElement>(`dialog [aria-label="${label}"]`)!;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(element, value);
      element.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  async function selectInstruments(symbols: string[] | null) {
    await act(async () => { [...container.querySelectorAll("button")].find((button) => button.textContent === "Select instruments")!.click(); });
    const click = async (text: string) => act(async () => { [...document.querySelectorAll<HTMLButtonElement>("dialog button")].find((button) => button.textContent === text)!.click(); });
    const dialog = document.querySelector("dialog")!;
    await act(async () => dialog.querySelectorAll<HTMLInputElement>('input[type="radio"]')[symbols === null ? 0 : 1]!.click());
    if (symbols !== null) {
      await click("Select none");
      for (const symbol of symbols) await act(async () => dialog.querySelector<HTMLInputElement>(`input[aria-label="${symbol}"]`)!.click());
    }
    await click("Apply");
  }
  const amount = (label: string) => container.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;
  const mode = (label: string, limited: boolean) => amount(label).closest("fieldset")!.querySelector<HTMLInputElement>(`input[type="radio"][value="${limited ? "limited" : "unlimited"}"]`)!;
  async function selectMode(label: string, limited: boolean) { await act(async () => mode(label, limited).click()); }
  const saveButton = () => [...container.querySelectorAll("button")].find((button) => button.textContent === "Save risk settings" || button.textContent === "Saving…");
  async function save() { await act(async () => { saveButton()!.click(); }); }
  it("requires exact name confirmation and navigates away after deletion", async () => {
    mocks.remove.mockResolvedValue({ deleted: true });
    const domains = ["subagents", "memories", "insights", "artifacts", "chart-instruments", "chart-layouts", "chart-history", "chart-fills"];
    for (const key of domains) {
      queryClient.setQueryData([key, initial.agent.id, "fixture"], []);
      queryClient.setQueryData([key, "other-agent", "fixture"], []);
    }
    const button = (text: string) => [...container.querySelectorAll("button")].find((button) => button.textContent === text)!;
    await act(async () => button("Delete agent").click());
    expect(button("Permanently delete agent").disabled).toBe(true);
    await change("Agent name confirmation", "research");
    expect(button("Permanently delete agent").disabled).toBe(true);
    await change("Agent name confirmation", initial.agent.name);
    await act(async () => button("Permanently delete agent").click());
    expect(mocks.remove).toHaveBeenCalledWith({ data: { id: initial.agent.id, confirmation: initial.agent.name } });
    expect(mocks.navigate).toHaveBeenCalledWith({ to: "/agents" });
    for (const key of domains) {
      expect(queryClient.getQueryData([key, initial.agent.id, "fixture"])).toBeUndefined();
      expect(queryClient.getQueryData([key, "other-agent", "fixture"])).toEqual([]);
    }
    expect(mocks.invalidate).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Existing exchange orders and positions are not changed");
  });
  it("preserves confirmation and supports retry after deletion fails", async () => {
    mocks.remove.mockRejectedValue(new Error("Deletion incomplete. Retry deletion."));
    const button = (text: string) => [...container.querySelectorAll("button")].find((button) => button.textContent === text)!;
    await act(async () => button("Delete agent").click());
    await change("Agent name confirmation", initial.agent.name);
    await act(async () => button("Permanently delete agent").click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Retry deletion");
    expect(button("Permanently delete agent").disabled).toBe(false);
    expect(mocks.navigate).not.toHaveBeenCalled();
  });
  it("only shows Save for changes, hides it on revert, and resets the baseline after saving", async () => {
    expect(saveButton()).toBeUndefined();
    const toggleTrading = async () => { await act(async () => container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click()); };
    await toggleTrading();
    expect(saveButton()).toBeDefined();
    await toggleTrading();
    expect(saveButton()).toBeUndefined();
    await selectInstruments(["BTC"]);
    expect(saveButton()).toBeDefined();
    await selectInstruments(null);
    expect(saveButton()).toBeUndefined();
    await selectInstruments(["BTC", "ETH"]);
    await save();
    expect(saveButton()).toBeUndefined();
    await selectInstruments(["ETH", "BTC"]);
    expect(saveButton()).toBeUndefined();
    await selectInstruments(["BTC"]);
    expect(saveButton()).toBeDefined();
    await selectInstruments(["BTC", "ETH"]);
    expect(saveButton()).toBeUndefined();
    expect(mocks.save).toHaveBeenCalledTimes(1);
  });
  it.each(["Max order notional", "Max total notional"])("shows Save for limit changes and hides it when the saved amount or mode is restored (%s)", async (label) => {
    await selectMode(label, true);
    expect(saveButton()).toBeDefined();
    await change(label, "100");
    await save();
    expect(saveButton()).toBeUndefined();
    await change(label, "200");
    expect(saveButton()).toBeDefined();
    await change(label, "100");
    expect(saveButton()).toBeUndefined();
    await selectMode(label, false);
    expect(saveButton()).toBeDefined();
    await selectMode(label, true);
    expect(saveButton()).toBeUndefined();
  });
  it("saves risk settings on the Settings page with explicit all-allowed and uncapped values", async () => {
    expect(container.querySelector('#risk-management')?.getAttribute("aria-label")).toBe("Risk management");
    expect(container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.checked).toBe(false);
    await selectInstruments(["BTC", "ETH"]);
    expect(mocks.save).not.toHaveBeenCalled();
    expect(container.querySelector('[aria-label="Allowed instruments"]')?.textContent).toContain("BTC");
    await selectMode("Max order notional", true);
    await selectMode("Max total notional", true);
    await change("Max order notional", "100.01");
    await change("Max total notional", "500");
    await act(async () => container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
    await save();
    expect(mocks.save).toHaveBeenLastCalledWith({ data: { agentId: initial.agent.id, settings: { tradingEnabled: true, allowedInstruments: ["BTC", "ETH"], maxOrderNotional: "100.01", maxTotalNotional: "500" } } });
    await selectInstruments(null);
    await selectMode("Max order notional", false);
    await selectMode("Max total notional", false);
    await save();
    expect(mocks.save).toHaveBeenLastCalledWith({ data: { agentId: initial.agent.id, settings: { tradingEnabled: true, allowedInstruments: null, maxOrderNotional: null, maxTotalNotional: null } } });
    expect(mocks.invalidate).not.toHaveBeenCalled();
  });
  it("defaults both limits to No limit with disabled amount inputs and USDC suffixes", async () => {
    for (const label of ["Max order notional", "Max total notional"]) {
      expect(mode(label, false).checked).toBe(true);
      expect(mode(label, true).checked).toBe(false);
      expect(amount(label).disabled).toBe(true);
      expect(amount(label).inputMode).toBe("decimal");
      expect(document.getElementById(amount(label).getAttribute("aria-describedby")!)?.textContent).toBe("USDC");
      expect(mode(label, true).name).toBe(mode(label, false).name);
    }
    expect(mode("Max order notional", true).name).not.toBe(mode("Max total notional", true).name);
    expect(saveButton()).toBeUndefined();
    expect(mocks.save).not.toHaveBeenCalled();
  });
  it("loads saved limits as selected Limit options without changing decimal precision", async () => {
    await act(async () => {
      queryClient.setQueryData(["risk", initial.agent.id], { tradingEnabled: true, allowedInstruments: null, maxOrderNotional: "100.00000001", maxTotalNotional: "500.25" });
      root.render(<AgentSettingsPage key="saved-limits" initial={initial} />);
    });
    expect(mode("Max order notional", true).checked).toBe(true);
    expect(mode("Max total notional", true).checked).toBe(true);
    expect(amount("Max order notional").disabled).toBe(false);
    expect(amount("Max total notional").disabled).toBe(false);
    expect(amount("Max order notional").value).toBe("100.00000001");
    expect(amount("Max total notional").value).toBe("500.25");
    expect(saveButton()).toBeUndefined();
  });
  it("keeps the options independent and preserves a limit draft when switching to No limit and back", async () => {
    await selectMode("Max order notional", true);
    await change("Max order notional", "100.01");
    expect(mode("Max total notional", false).checked).toBe(true);
    await selectMode("Max order notional", false);
    expect(amount("Max order notional").disabled).toBe(true);
    expect(amount("Max order notional").value).toBe("100.01");
    expect(saveButton()).toBeUndefined();
    expect(mocks.save).not.toHaveBeenCalled();
    await selectMode("Max order notional", true);
    expect(amount("Max order notional").value).toBe("100.01");
    await save();
    expect(mocks.save.mock.calls[0]![0].data.settings).toMatchObject({ maxOrderNotional: "100.01", maxTotalNotional: null });
  });
  it.each(["Max order notional", "Max total notional"])("rejects an empty selected limit instead of silently removing it (%s)", async (label) => {
    await selectMode(label, true);
    await change(label, "  ");
    await save();
    expect(mocks.save).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')!.textContent).toContain("limit or select No limit");
    expect(mode(label, true).checked).toBe(true);
    expect(amount(label).value).toBe("  ");
  });
  it("disables both limit selectors and amounts while saving", async () => {
    let resolve!: (value: unknown) => void;
    mocks.save.mockImplementation(() => new Promise((done) => { resolve = done; }));
    await selectMode("Max order notional", true);
    await change("Max order notional", "100");
    await save();
    for (const label of ["Max order notional", "Max total notional"]) {
      expect(mode(label, false).matches(":disabled")).toBe(true);
      expect(mode(label, true).matches(":disabled")).toBe(true);
      expect(amount(label).matches(":disabled")).toBe(true);
    }
    expect(saveButton()?.disabled).toBe(true);
    expect([...container.querySelectorAll("button")].find((button) => button.textContent === "Select instruments")?.disabled).toBe(true);
    await act(async () => resolve({ tradingEnabled: false, allowedInstruments: null, maxOrderNotional: "100", maxTotalNotional: null }));
    expect(saveButton()).toBeUndefined();
    expect(mode("Max order notional", false).matches(":disabled")).toBe(false);
  });
  it("styles the trading checkbox for the dark theme while retaining a native labeled input", () => {
    const checkbox = container.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    expect(checkbox.labels?.[0]?.textContent?.trim()).toBe("Enable Trading");
    expect(checkbox.classList.contains("appearance-none")).toBe(true);
    expect(checkbox.classList.contains("checked:bg-violet-600")).toBe(true);
    expect(checkbox.classList.contains("focus-visible:ring-2")).toBe(true);
    expect(checkbox.parentElement?.querySelector('svg[aria-hidden="true"]')?.classList.contains("peer-checked:opacity-100")).toBe(true);
  });
  it("preserves the draft when saving fails", async () => {
    mocks.save.mockRejectedValue(new Error("Unable to save"));
    await selectMode("Max total notional", true);
    await change("Max total notional", "123.45");
    await save();
    expect(container.querySelector('[role="alert"]')!.textContent).toContain("Unable to save");
    expect(saveButton()).toBeDefined();
    expect(saveButton()?.disabled).toBe(false);
    expect(mocks.invalidate).not.toHaveBeenCalled();
    expect(container.querySelector<HTMLInputElement>('[aria-label="Max total notional"]')!.value).toBe("123.45");
  });
  it("keeps application settings without orchestrator editing controls", () => {
    expect(container.textContent).not.toContain("Execution backend"); expect(container.textContent).toContain("Telegram");
    expect(container.querySelector("select")).toBeNull();
    expect(container.textContent).not.toContain("Edit orchestrator model");
    expect(eventSource).not.toHaveBeenCalled();
  });
  it("keeps application settings without orchestrator editing controls", () => {
    expect(container.textContent).not.toContain("Execution backend"); expect(container.textContent).toContain("Telegram");
    expect(container.querySelector("select")).toBeNull();
    expect(container.textContent).not.toContain("Edit orchestrator model");
    // The setup wizard and its links live in the dialog, never in the inline section.
    expect(container.querySelector("a")).toBeNull();
    expect(eventSource).not.toHaveBeenCalled();
  });
  it("hides notification controls until Telegram is connected", async () => {
    expect(container.querySelector("#notifications")).toBeNull();
    await act(async () => queryClient.setQueryData(["gateway", initial.agent.id], { hasToken: true, connected: false, botUsername: "my_bot", chatId: null, chatUsername: null }));
    expect(container.querySelector("#notifications")).toBeNull();
  });
  it("groups notification controls inside connected Telegram without descriptive subtext", async () => {
    await act(async () => queryClient.setQueryData(["gateway", initial.agent.id], { hasToken: true, connected: true, botUsername: "my_bot", chatId: 7, chatUsername: "operator" }));
    const telegram = container.querySelector('section[aria-label="Telegram"]')!;
    const notifications = telegram.querySelector('section[aria-label="Notifications"]')!;
    expect(notifications.id).toBe("notifications");
    expect(notifications.querySelector("h3")?.textContent).toBe("Notifications");
    expect(notifications.querySelector("input")?.labels?.[0]?.textContent).toBe("Enable Telegram notifications");
    expect(telegram.querySelector("h2")?.nextElementSibling?.tagName).toBe("DIV");
    expect(notifications.querySelector("h3")?.nextElementSibling?.tagName).toBe("FORM");
    expect(container.querySelectorAll('section[aria-label="Notifications"]')).toHaveLength(1);
    expect(telegram.textContent).not.toContain("Bridge this Agent");
  });
  it.each(["stopping", "stopped"] as const)("hides notification controls for %s agents but keeps Telegram available", async (state) => {
    await act(async () => queryClient.setQueryData(["gateway", initial.agent.id], { hasToken: true, connected: true, botUsername: "my_bot", chatId: 7, chatUsername: "operator" }));
    await act(async () => root.render(<AgentSettingsPage initial={{ ...initial, agent: { ...initial.agent, state } }} />));
    const telegram = container.querySelector('section[aria-label="Telegram"]')!;
    expect(telegram).not.toBeNull();
    expect(telegram.textContent).toContain("Disconnect Telegram");
    expect(container.querySelector("#notifications")).toBeNull();
    expect(container.querySelector("#risk-management")).toBeNull();
  });
  it("keeps connected notification controls available when the agent is disabled", async () => {
    await act(async () => queryClient.setQueryData(["gateway", initial.agent.id], { hasToken: true, connected: true, botUsername: "my_bot", chatId: 7, chatUsername: "operator" }));
    await act(async () => root.render(<AgentSettingsPage initial={{ ...initial, agent: { ...initial.agent, enabled: false } }} />));
    expect(container.querySelector('#telegram #notifications input[type="checkbox"]')).not.toBeNull();
  });
  it("completes the Telegram setup flow inside the connect dialog", async () => {
    const configured = { hasToken: true, connected: false, botUsername: "my_bot", chatId: null, chatUsername: null };
    mocks.addToken.mockResolvedValue(configured);
    mocks.startLink.mockResolvedValue({ link: "https://t.me/my_bot?start=abc-123" });
    await act(async () => [...container.querySelectorAll("button")].find((button) => button.textContent === "Connect Telegram")!.click());
    const dialog = () => document.querySelector("dialog")!;
    expect(dialog().getAttribute("aria-labelledby")).toBeDefined();
    expect(dialog().textContent).toContain("@BotFather");
    await dialogChange("Telegram bot token", "123456789:AAE-fixture");
    await act(async () => [...dialog().querySelectorAll("button")].find((button) => button.textContent === "Add bot token")!.click());
    expect(mocks.addToken).toHaveBeenCalledWith({ data: { id: initial.agent.id, token: "123456789:AAE-fixture" } });
    expect(mocks.success).toHaveBeenCalledWith("Telegram bot added");
    expect(dialog().textContent).toContain("@my_bot");
    expect(queryClient.getQueryData(["gateway", initial.agent.id])).toEqual(configured);
    await act(async () => [...dialog().querySelectorAll("button")].find((button) => button.textContent === "Link Telegram chat")!.click());
    expect(mocks.startLink).toHaveBeenCalledWith({ data: { id: initial.agent.id } });
    const anchor = [...dialog().querySelectorAll<HTMLAnchorElement>("a")].find((anchor) => anchor.href === "https://t.me/my_bot?start=abc-123")!;
    expect(anchor.target).toBe("_blank");
    expect(anchor.className).toContain("cursor-pointer");
    expect(dialog().querySelector('[role="alert"]')).toBeNull();
    await act(async () => [...dialog().querySelectorAll("button")].find((button) => button.textContent === "Close")!.click());
    expect(document.querySelector("dialog")).toBeNull();
    expect(container.querySelector("a")).toBeNull();
  });
  it("advances to connected when the linked chat arrives while the dialog is open", async () => {
    vi.useFakeTimers();
    try {
      await act(async () => queryClient.setQueryData(["gateway", initial.agent.id], { hasToken: true, connected: false, botUsername: "my_bot", chatId: null, chatUsername: null }));
      mocks.status.mockResolvedValue({ hasToken: true, connected: true, botUsername: "my_bot", chatId: 7, chatUsername: "operator" });
      await act(async () => [...container.querySelectorAll("button")].find((button) => button.textContent === "Connect Telegram")!.click());
      await act(async () => { await vi.advanceTimersByTimeAsync(4000); });
      expect(document.querySelector("dialog")!.textContent).toContain("Telegram connected");
      expect(document.querySelector("dialog")!.textContent).toContain("@operator");
      // The parent section stays in sync with the connected chat.
      await act(async () => {});
      expect(queryClient.getQueryData(["gateway", initial.agent.id])).toMatchObject({ connected: true, chatId: 7 });
      expect(container.querySelector("#notifications h3")?.textContent).toBe("Notifications");
    } finally { vi.useRealTimers(); }
  });
  it("shows the connected chat and disconnects Telegram", async () => {
    await act(async () => queryClient.setQueryData(["gateway", initial.agent.id], { hasToken: true, connected: true, botUsername: "my_bot", chatId: 7, chatUsername: "operator" }));
    expect(container.textContent).toContain("@my_bot");
    expect(container.textContent).toContain("@operator");
    expect(container.textContent).toContain("7");
    const removed = { hasToken: false, connected: false, botUsername: null, chatId: null, chatUsername: null };
    mocks.disconnect.mockResolvedValue(removed);
    await act(async () => [...container.querySelectorAll("button")].find((button) => button.textContent === "Disconnect Telegram")!.click());
    expect(mocks.disconnect).toHaveBeenCalledWith({ data: { id: initial.agent.id } });
    expect(queryClient.getQueryData(["gateway", initial.agent.id])).toEqual(removed);
    expect(container.querySelector("#notifications")).toBeNull();
    expect([...container.querySelectorAll("button")].find((button) => button.textContent === "Connect Telegram")).toBeDefined();
  });
  it("reports Telegram failures inside the dialog without losing the draft", async () => {
    mocks.addToken.mockRejectedValue(new Error("Telegram rejected this bot token"));
    await act(async () => [...container.querySelectorAll("button")].find((button) => button.textContent === "Connect Telegram")!.click());
    const dialog = () => document.querySelector("dialog")!;
    await dialogChange("Telegram bot token", "bad-token");
    await act(async () => [...dialog().querySelectorAll("button")].find((button) => button.textContent === "Add bot token")!.click());
    expect(dialog().querySelector('[role="alert"]')!.textContent).toContain("Telegram rejected this bot token");
    expect(dialog().querySelector<HTMLInputElement>('[aria-label="Telegram bot token"]')!.value).toBe("bad-token");
  });
  it("surfaces a polling error from the gateway status", async () => {
    await act(async () => queryClient.setQueryData(["gateway", initial.agent.id], { hasToken: true, connected: true, botUsername: "my_bot", chatId: 7, chatUsername: "operator", error: "401: Unauthorized: token revoked" }));
    expect(container.querySelector('#telegram [role="status"]')!.textContent).toContain("token revoked");
  });
});
