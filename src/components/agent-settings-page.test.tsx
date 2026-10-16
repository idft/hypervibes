// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentChat } from "../agents/agent-service.js";
import { AgentSettingsPage } from "./agent-settings-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const initial: AgentChat = {
  agent: { id: "agent-fixture", name: "Research", userId: "owner", state: "ready", conversationId: 1, initialProvider: null, initialModelId: null, error: null, tradingAccountAddress: null, creationRequestId: "fixture", createdAt: 0, updatedAt: 0 },
  model: { provider: "current", modelId: "old" }, configured: true, busy: false, queued: 0, messages: [], partial: "",
};
describe("Agent settings", () => {
  let root: Root; let container: HTMLDivElement;
  const eventSource = vi.fn();
  beforeEach(async () => {
    vi.stubGlobal("EventSource", eventSource);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<AgentSettingsPage initial={initial} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
  it("keeps application settings and links to the protected orchestrator instead of selecting models here", () => {
    expect(container.textContent).toContain("Execution backend"); expect(container.textContent).toContain("Telegram");
    expect(container.querySelector("select")).toBeNull();
    expect(container.querySelector("a")?.getAttribute("href")).toBe("/agents/agent-fixture/sub-agents/main%3Aagent-fixture");
    expect(eventSource).not.toHaveBeenCalled();
  });
});
