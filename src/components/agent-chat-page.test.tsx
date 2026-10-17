// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeAll, afterAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentChat } from "../agents/agent-service.js";
import type { ModelChoice } from "../providers/provider-service.js";

const mocks = vi.hoisted(() => ({ compactAgentChat: vi.fn(), getAgentChatHistory: vi.fn(), retryAgent: vi.fn(), resetAgentChat: vi.fn(), saveSubagentSection: vi.fn(), sendAgentMessage: vi.fn(), stopAgentChat: vi.fn() }));
vi.mock("../web/agents.js", () => mocks);
vi.mock("../web/subagents.js", () => ({ saveSubagentSection: mocks.saveSubagentSection }));
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
  model: { provider: "opencode-go", modelId: "deepseek-v4.1-flash" }, configured: true, busy: false, queuedMessages: [], messages: [], partial: "",
  usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0 }, history: false, compacting: false,
};
const choice = (modelId: string, name: string, thinkingLevels: string[], contextWindow = 128_000): ModelChoice => ({
  provider: "opencode-go", modelId, name, providerName: "OpenCode", thinkingLevels, contextWindow, reasoning: thinkingLevels.length > 1, inputCost: 0.1, outputCost: 0.4,
});
const choices = [choice("deepseek-v4.1-flash", "DeepSeek Flash", ["", "low", "high"]), choice("gpt-test", "GPT Test", [])];
describe("Agent chat", () => {
  let container: HTMLDivElement;
  let root: Root;
  const originalDialogMethods = Object.fromEntries(["showModal", "close"].map((key) => [key, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, key)]));
  beforeAll(() => {
    // jsdom does not implement the native dialog lifecycle.
    Object.defineProperties(HTMLDialogElement.prototype, {
      showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
      close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
    });
  });
  afterAll(() => {
    for (const [key, descriptor] of Object.entries(originalDialogMethods)) {
      if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, key, descriptor);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, key);
    }
  });
  beforeEach(async () => {
    vi.resetAllMocks();
    FakeEventSource.instances = [];
    Element.prototype.scrollIntoView = vi.fn();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => root.render(<AgentChatPage initial={initial} choices={choices} />));
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
    const html = renderToStaticMarkup(<AgentChatPage initial={{ ...initial, messages: [{ id: "one", role: "assistant", text: "<script>bad()</script>", failed: false }] }} choices={choices} />);
    expect(html).toContain("DeepSeek Flash");
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
  it("shows the title once and changes the model from the header picker", async () => {
    const header = () => container.querySelector("header")!;
    expect(header().textContent.match(/Main conversation/g)?.length).toBe(1);
    expect(container.textContent).not.toContain("New conversation");
    expect(header().querySelectorAll("button").length).toBe(3);
    const trigger = header().querySelector('button[aria-label="Choose model"]') as HTMLButtonElement;
    expect(trigger.textContent).toContain("DeepSeek Flash");
    await act(async () => current().emit("chat", { ...initial, thinkingLevel: "high" }));
    expect(trigger.textContent).toContain("Thinking: high");
    await act(async () => { trigger.click(); });
    const dialog = document.querySelector("dialog")!;
    expect(dialog.open).toBe(true);
    expect(dialog.textContent).toContain("GPT Test");
    await act(async () => {
      const radio = [...dialog.querySelectorAll<HTMLInputElement>("input[name=model]")].find((input) => input.closest("label")!.textContent!.includes("GPT Test"))!;
      radio.click();
    });
    mocks.saveSubagentSection.mockResolvedValueOnce({ ok: true, data: {} });
    await act(async () => { [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Apply")!.click(); });
    await act(async () => {});
    expect(mocks.saveSubagentSection).toHaveBeenCalledWith({ data: { agentId: "agent-fixture", id: "main:agent-fixture", section: "model", settings: { provider: "opencode-go", modelId: "gpt-test", thinkingLevel: "" } } });
    expect(document.querySelector("dialog")).toBeNull();
    // A new model is ready for typing: focus returns to the composer.
    expect(document.activeElement).toBe(container.querySelector("textarea"));
  });
  it("disables the model picker while the orchestrator is working", async () => {
    await act(async () => current().emit("chat", { ...initial, busy: true, queuedMessages: [{ id: "inbox:1", text: "Follow up" }] }));
    const trigger = container.querySelector('button[aria-label="Choose model"]') as HTMLButtonElement;
    expect(trigger.hasAttribute("disabled")).toBe(true);
    expect(trigger.title).toContain("Stop or finish queued work");
  });
  it("disables chat when credentials are missing", async () => {
    await act(async () => root.render(<AgentChatPage initial={{ ...initial, configured: false }} choices={choices} key="unconfigured" />));
    await type("Hello");
    expect(container.textContent).toContain("Reconnect this Agent’s provider");
    expect(container.querySelector("form button")!.hasAttribute("disabled")).toBe(true);
  });
  it("sends on Enter and keeps Shift+Enter as a newline", async () => {
    mocks.sendAgentMessage.mockResolvedValue({ ok: true, data: { submissionId: 1 } });
    const textarea = () => container.querySelector("textarea")!;
    const key = (options: KeyboardEventInit) => textarea().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...options }));
    await type("Hello");
    await act(async () => { key({ key: "Enter", shiftKey: true }); });
    expect(mocks.sendAgentMessage).not.toHaveBeenCalled();
    await act(async () => { expect(key({ key: "Enter" })).toBe(false); });
    await act(async () => {});
    expect(mocks.sendAgentMessage).toHaveBeenCalledOnce();
    expect(textarea().value).toBe("");
  });
  it("renders the context meter and conversation token totals", () => {
    const html = renderToStaticMarkup(<AgentChatPage initial={{
      ...initial,
      context: { tokens: 38_000, window: 200_000 },
      usage: { input: 12_400, output: 3_100, cacheRead: 800, cacheWrite: 0, cost: 0.0042 },
    }} choices={choices} />);
    expect(html).toContain("19%");
    expect(html).toContain("38K / 200K tokens");
    expect(html).toContain("12.4K in · 3.1K out · $0.0042");
  });
  it("requires confirmation before starting a new chat and can compact", async () => {
    mocks.resetAgentChat.mockResolvedValue({ ok: true });
    mocks.compactAgentChat.mockResolvedValue({ ok: true, data: {} });
    const buttons = () => [...container.querySelectorAll("header button")] as [HTMLButtonElement, HTMLButtonElement, HTMLButtonElement];
    const [, compactButton, resetButton] = buttons();
    await act(async () => { compactButton.click(); });
    expect(mocks.compactAgentChat).toHaveBeenCalledOnce();
    await act(async () => { resetButton.click(); });
    expect(mocks.resetAgentChat).not.toHaveBeenCalled();
    expect(resetButton.textContent).toContain("Confirm new chat?");
    await act(async () => { resetButton.click(); });
    expect(mocks.resetAgentChat).toHaveBeenCalledOnce();
    // A fresh conversation is ready for typing: focus returns to the composer.
    expect(document.activeElement).toBe(container.querySelector("textarea"));
  });
  it("shows the history hint only when earlier messages exist", async () => {
    expect(container.textContent).not.toContain("Scroll up to load earlier messages");
    await act(async () => current().emit("chat", { ...initial, history: true }));
    expect(container.textContent).toContain("Scroll up to load earlier messages");
    await act(async () => current().emit("chat", initial));
    expect(container.textContent).not.toContain("Scroll up to load earlier messages");
  });
  it("loads older transcript pages when scrolled to the top", async () => {
    mocks.getAgentChatHistory.mockResolvedValue({ ok: true, data: { messages: [
      { id: "old:1", role: "user", text: "First ever", failed: false },
      { id: "old:2", role: "assistant", text: "Ancient reply", failed: false },
    ], before: 2, more: false } });
    await act(async () => container.querySelector("[aria-label=Conversation]")!.dispatchEvent(new Event("scroll", { bubbles: true })));
    expect(mocks.getAgentChatHistory).toHaveBeenCalledWith({ data: { id: "agent-fixture", before: undefined } });
    expect(container.textContent).toContain("First ever");
    expect(container.textContent).toContain("Ancient reply");
    expect(container.textContent).not.toContain("Scroll up to load earlier messages");
    expect(container.textContent).not.toContain("Loading earlier messages…");
  });
  it("drops loaded history when a new conversation starts", async () => {
    mocks.getAgentChatHistory.mockResolvedValue({ ok: true, data: { messages: [
      { id: "old:1", role: "user", text: "From the old session", failed: false },
    ], before: 5, more: false } });
    await act(async () => container.querySelector("[aria-label=Conversation]")!.dispatchEvent(new Event("scroll", { bubbles: true })));
    expect(container.textContent).toContain("From the old session");
    await act(async () => current().emit("chat", {
      ...initial,
      messages: [{ id: "reset:9", role: "system", text: "New conversation", failed: false, marker: "reset" }, { id: "10:0", role: "assistant", text: "Fresh context", failed: false }],
    }));
    expect(container.textContent).not.toContain("From the old session");
    expect(container.textContent).toContain("New conversation");
    expect(container.textContent).toContain("Fresh context");
  });
  it("restores composer focus after a send so chatting stays hands-on-keyboard", async () => {
    mocks.sendAgentMessage.mockResolvedValue({ ok: true, data: { submissionId: 1 } });
    const textarea = () => container.querySelector("textarea")!;
    expect(document.activeElement).toBe(textarea());
    await act(async () => { (document.activeElement as HTMLElement).blur(); });
    await type("Hello");
    await submit();
    await act(async () => {});
    expect(mocks.sendAgentMessage).toHaveBeenCalledOnce();
    expect(document.activeElement).toBe(textarea());
  });
  it("renders queued messages as bubbles below the streaming response", async () => {
    await act(async () => current().emit("chat", {
      ...initial,
      busy: true,
      partial: "Still streaming…",
      queuedMessages: [{ id: "inbox:1", text: "Follow up question" }, { id: "inbox:2", text: "And another thing" }],
    }));
    expect(container.textContent).toContain("Still streaming…");
    expect(container.textContent).toContain("Follow up question");
    expect(container.textContent).toContain("And another thing");
    expect(container.textContent).toContain("2 queued");
    expect(container.textContent).toContain("Queued · sent when the response finishes");
  });
  it("explains when there is nothing to compact", async () => {
    mocks.compactAgentChat.mockResolvedValue({ ok: true, data: { message: "Nothing to compact yet — the recent conversation still fits within the context window." } });
    const compactButton = container.querySelectorAll("header button")[1] as HTMLButtonElement;
    await act(async () => { compactButton.click(); });
    expect(container.textContent).toContain("Nothing to compact yet — the recent conversation still fits within the context window.");
    // A compaction marker arriving in the transcript answers an ongoing compaction.
    await act(async () => current().emit("chat", { ...initial, messages: [{ id: "compaction:5", role: "system", text: "Earlier messages were compacted into a summary", failed: false, marker: "compaction" }] }));
    expect(container.textContent).not.toContain("Nothing to compact yet");
  });
  it("shows live compaction status", async () => {
    await act(async () => current().emit("chat", { ...initial, compacting: true, compactionError: "Summarization failed: model unavailable" }));
    expect(container.textContent).toContain("Compacting…");
    expect(container.textContent).toContain("Compaction failed: Summarization failed: model unavailable");
    const compactButton = container.querySelectorAll("header button")[1] as HTMLButtonElement;
    expect(compactButton.hasAttribute("disabled")).toBe(true);
  });
});