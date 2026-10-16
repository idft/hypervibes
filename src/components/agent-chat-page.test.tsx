// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentChat } from "../agents/agent-service.js";

const mocks = vi.hoisted(() => ({ retryAgent: vi.fn(), selectAgentModel: vi.fn(), sendAgentMessage: vi.fn(), stopAgentChat: vi.fn() }));
vi.mock("../web/agents.js", () => mocks);
import { AgentChatPage } from "./agent-chat-page.js";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

class FakeEventSource {
  static instances: FakeEventSource[] = [];
  listeners = new Map<string, ((event: { data: string }) => void)[]>();
  onerror: (() => void) | null = null;
  url: string;
  closed = false;
  constructor(url: string) {
    this.url = url;
    FakeEventSource.instances.push(this);
  }
  addEventListener(type: string, listener: (event: { data: string }) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  emit(type: string, data: unknown) {
    for (const listener of this.listeners.get(type) ?? []) listener({ data: JSON.stringify(data) });
  }
  error() { this.onerror?.(); }
  close() { this.closed = true; }
}
vi.stubGlobal("EventSource", FakeEventSource);

const initial: AgentChat = {
  agent: { id: "agent-fixture", name: "Research", userId: "owner", state: "ready", conversationId: 1, initialProvider: null, initialModelId: null, error: null, tradingAccountAddress: null, creationRequestId: "fixture", createdAt: 0, updatedAt: 0 },
  model: { provider: "opencode-go", modelId: "deepseek-v4.1-flash" }, configured: true, busy: false, queued: 0, messages: [], partial: "",
};
describe("Agent chat", () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(async () => {
    vi.resetAllMocks();
    FakeEventSource.instances = [];
    Element.prototype.scrollIntoView = vi.fn();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => root.render(<AgentChatPage initial={initial} />));
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });
  const current = () => FakeEventSource.instances.at(-1)!;
  async function type(text: string) {
    const textarea = container.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, text);
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  async function submit() {
    await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  }
  it("subscribes once per agent and closes the stream on unmount", async () => {
    expect(current().url).toBe("/api/agents/agent-fixture/events");
    expect(current().closed).toBe(false);
    await act(async () => root.unmount());
    expect(current().closed).toBe(true);
    root = createRoot(container);
  });
  it("renders escaped messages, model and an honest no-trading boundary", () => {
    const html = renderToStaticMarkup(<AgentChatPage initial={{ ...initial, messages: [{ id: "one", role: "assistant", text: "<script>bad()</script>", failed: false }] }} />);
    expect(html).toContain("deepseek-v4.1-flash");
    expect(html).not.toContain("Enabled order tools");
    expect(html).toContain("&lt;script&gt;");
    expect(html).not.toContain("<script>");
  });
  it("applies pushed snapshots without polling requests", async () => {
    mocks.sendAgentMessage.mockResolvedValue({ ok: true, data: { submissionId: 1 } });
    await type("Hello");
    await submit();
    expect(mocks.sendAgentMessage).toHaveBeenCalledOnce();
    await act(async () => current().emit("chat", { ...initial, busy: true, partial: "A partial answer" }));
    expect(container.textContent).toContain("A partial answer");
    expect(container.textContent).toContain("Stop response");
    await act(async () => current().emit("chat", { ...initial, messages: [{ id: "one", role: "assistant", text: "Finished", failed: false }], busy: false }));
    expect(container.textContent).toContain("Finished");
    expect(container.textContent).toContain("Idle");
  });
  it("reports stream failures and expired sessions without duplicating alerts", async () => {
    await act(async () => current().emit("unavailable", { message: "Chat stream interrupted. Reconnecting…" }));
    expect(container.querySelector("[role=alert]")?.textContent).toContain("Chat stream interrupted");
    await act(async () => current().emit("chat", initial));
    expect(container.querySelector("[role=alert]")).toBeNull();
    const location = window.location;
    vi.spyOn(window, "location", "get").mockReturnValue({ ...location, assign: vi.fn() } as unknown as Location);
    await act(async () => current().emit("unauthorized", { message: "Session ended" }));
    expect(window.location.assign).toHaveBeenCalledWith("/login");
    vi.restoreAllMocks();
    root = createRoot(container);
  });
  it("preserves the request ID after uncertain failure and prevents overlapping sends", async () => {
    let reject!: (error: Error) => void;
    mocks.sendAgentMessage.mockImplementation(() => new Promise((_resolve, fail) => { reject = fail; }));
    await type("Hello");
    await submit();
    await submit();
    expect(mocks.sendAgentMessage).toHaveBeenCalledOnce();
    const first = mocks.sendAgentMessage.mock.calls[0]![0];
    expect(first.data.message).toBe("Hello");
    await act(async () => reject(new Error("network failure")));
    expect(container.querySelector("[role=alert]")?.textContent).toContain("Unable to complete");
    mocks.sendAgentMessage.mockResolvedValue({ ok: true, data: { submissionId: 1 } });
    await submit();
    expect(mocks.sendAgentMessage.mock.calls[1]![0]).toEqual(first);
    expect(container.querySelector("textarea")!.value).toBe("");
  });
  it("keeps model selection out of chat and has only one conversation", () => {
    expect(container.querySelector("select")).toBeNull();
    expect(container.textContent).toContain("Main conversation");
    expect(container.textContent).not.toContain("New conversation");
  });
  it("disables chat when credentials are missing", async () => {
    await act(async () => root.render(<AgentChatPage initial={{ ...initial, configured: false }} key="unconfigured" />));
    await type("Hello");
    expect(container.textContent).toContain("Reconnect this Agent’s provider");
    expect(container.querySelector("form button")!.hasAttribute("disabled")).toBe(true);
  });
});