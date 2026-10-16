// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentChat } from "../agents/agent-service.js";

const mocks = vi.hoisted(() => ({ getAgentChat: vi.fn(), selectAgentModel: vi.fn() }));
vi.mock("../web/agents.js", () => mocks);
import { AgentSettingsPage } from "./agent-settings-page.js";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const initial: AgentChat = {
  agent: { id: "agent-fixture", name: "Research", userId: "owner", state: "ready", conversationId: 1, initialProvider: null, initialModelId: null, error: null, tradingAccountAddress: null, creationRequestId: "fixture", createdAt: 0, updatedAt: 0 },
  model: { provider: "current", modelId: "old" }, configured: true, busy: false, queued: 0, messages: [], partial: "",
};
describe("Agent settings", () => {
  let root: Root;
  let container: HTMLDivElement;
  const eventSource = vi.fn();
  beforeEach(async () => {
    vi.resetAllMocks();
    vi.stubGlobal("EventSource", eventSource);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    mocks.getAgentChat.mockResolvedValue({ ok: true, data: initial });
    await act(async () => root.render(<AgentSettingsPage initial={initial} choices={[
      { ...initial.model, name: "Current", thinkingLevels: [] },
      { provider: "other", modelId: "new", name: "New model", thinkingLevels: ["high"] },
    ]} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  it("shows future settings without opening a chat connection", () => {
    expect(container.textContent).toContain("Execution backend");
    expect(container.textContent).toContain("Telegram");
    expect(container.textContent).not.toContain("Capabilities");
    expect(eventSource).not.toHaveBeenCalled();
  });
  it("saves model and thinking, then refreshes once without subscribing", async () => {
    mocks.selectAgentModel.mockResolvedValue({ ok: true });
    const selects = () => container.querySelectorAll("select");
    await act(async () => {
      selects()[0]!.value = JSON.stringify(["other", "new"]);
      selects()[0]!.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => {
      selects()[1]!.value = "high";
      selects()[1]!.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => container.querySelector("button")!.click());
    expect(mocks.selectAgentModel).toHaveBeenCalledWith({ data: { id: initial.agent.id, provider: "other", modelId: "new", thinkingLevel: "high" } });
    expect(mocks.getAgentChat).toHaveBeenCalledOnce();
    expect(container.textContent).toContain("Model updated");
    expect(eventSource).not.toHaveBeenCalled();
  });
  it("shows the server's idle-state rejection and permits retry without a live stream", async () => {
    mocks.selectAgentModel.mockResolvedValue({ ok: false, error: "Wait until the Agent is idle" });
    await act(async () => container.querySelector("button")!.click());
    expect(container.querySelector("[role=alert]")?.textContent).toContain("idle");
    expect(container.querySelector("button")!.disabled).toBe(false);
    expect(eventSource).not.toHaveBeenCalled();
  });
});
