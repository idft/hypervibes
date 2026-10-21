// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { QueryClient, QueryClientProvider, notifyManager } from "@tanstack/react-query";
import { fauxAssistantMessage } from "@earendil-works/pi-ai/providers/faux";
import { createRoot, type Root } from "react-dom/client";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RunDetail } from "../agents/subagent-service.js";
import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";
import type { EntryRecord } from "@earendil-works/pi-durable";
const mocks = vi.hoisted(() => ({ stopSubagentRun: vi.fn() }));
vi.mock("../web/subagents.js", () => mocks);
import { SubagentRunHistory } from "./subagent-run-history.js";
import { SubagentRunPage as SubagentRunPageComponent } from "./subagent-run-page.js";
notifyManager.setScheduler((callback) => { queueMicrotask(callback); return () => {}; });
let client: QueryClient;
function SubagentRunPage(props: ComponentProps<typeof SubagentRunPageComponent>) {
  return <QueryClientProvider client={client}><SubagentRunPageComponent {...props} /></QueryClientProvider>;
}
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const definition: SubagentRow = { id: "worker:fixture", agentId: "agent-fixture", kind: "worker", name: "Research", description: "", instructions: "Saved worker instructions", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], triggers: { inboxEnabled: false, workerDelegates: [], researchSources: [], fills: false, orderChanges: false }, templateOrigin: null, enabled: true, archived: false, delegatable: true, persistent: true, conversationId: null, schedule: "", delaySeconds: 30, nextRunAt: null, completion: "save", timeoutSeconds: 900, createdAt: 1, updatedAt: 1 };
const run: SubagentRun = { id: "large-hex-id", agentId: definition.agentId, subagentId: definition.id, requestId: "request", source: "manual", task: "Full assignment prompt", config: definition, status: "completed", taskId: null, conversationId: null, result: "Complete final result", createdAt: Date.now() - 300000, finishedAt: Date.now(), reportedAt: null };
const entry = (id: number, kind: string, model: EntryRecord["model"]) => ({ id, kind, conversationId: 1, model }) as EntryRecord;
const initial = {
  definition: { ...definition, instructions: "Changed since execution" }, run,
  prior: [entry(1, "pi.user", [{ role: "user", content: "Earlier assignment", timestamp: 1 }])],
  entries: [entry(2, "pi.user", [{ role: "user", content: run.task, timestamp: 1 }]), entry(3, "pi.assistant", [fauxAssistantMessage([{ type: "thinking", thinking: "Reasoning evidence" }, { type: "text", text: "Full assistant response" }, { type: "toolCall", id: "call", name: "get_account", arguments: { coin: "BTC" } }], { stopReason: "toolUse", timestamp: 1 })]), entry(4, "pi.tool", [{ role: "toolResult", toolCallId: "call", toolName: "get_account", content: [{ type: "text", text: "Complete tool output" }], isError: false, timestamp: 1 }])],
};
class RunSource extends EventTarget {
  static instances: RunSource[] = [];
  onerror: (() => void) | null = null;
  close = vi.fn();
  constructor(readonly url: string) { super(); RunSource.instances.push(this); }
}
describe("sub-agent run history and details", () => {
  let root: Root, container: HTMLDivElement;
  const dialogMethods = Object.fromEntries(["showModal", "close"].map((key) => [key, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, key)]));
  beforeEach(() => { vi.resetAllMocks(); client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } }); Object.defineProperties(HTMLDialogElement.prototype, {
    showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
    close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
  }); RunSource.instances = []; vi.stubGlobal("EventSource", RunSource); container = document.createElement("div"); container.className = "app-shell"; Element.prototype.scrollIntoView = vi.fn(); document.body.append(container); root = createRoot(container); });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); client.clear(); vi.restoreAllMocks(); vi.unstubAllGlobals(); for (const [key, descriptor] of Object.entries(dialogMethods)) { if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, key, descriptor); else Reflect.deleteProperty(HTMLDialogElement.prototype, key); } });
  it("renders a table with relative timestamps, accessible status icons, links and isolated Stop actions", async () => {
    const stop = vi.fn();
    await act(async () => root.render(<SubagentRunHistory agentId={run.agentId} runs={[run, { ...run, id: "running-id", status: "running", source: "delegation" }]} pending={false} onStop={stop} />));
    expect(container.querySelectorAll("tbody tr")).toHaveLength(2);
    expect([...container.querySelectorAll("thead th")].map((cell) => cell.textContent)).toEqual(["Created", "Source", "Instruction", "Status", "Actions"]);
    expect(container.querySelector("tbody tr td:nth-child(4) [role=img]")?.getAttribute("aria-label")).toBe("completed");
    expect(container.querySelector("tbody td time")).not.toBeNull();
    expect(container.textContent).not.toContain(run.id);
    expect(container.querySelector('[role="img"]')?.getAttribute("aria-label")).toBe("completed");
    expect(container.querySelector("time")?.textContent).toBe("5 minutes ago");
    expect(container.querySelector("time")?.title).toBe(new Date(run.createdAt).toISOString());
    const link = container.querySelector("a")!;
    expect(link.getAttribute("href")).toBe("/agents/agent-fixture/sub-agents/worker%3Afixture/runs/large-hex-id");
    expect(link.className).toContain("truncate");
    const navigate = vi.fn(); link.addEventListener("click", (event) => { event.preventDefault(); navigate(); });
    await act(async () => container.querySelector("td")!.click());
    expect(navigate).toHaveBeenCalledOnce();
    await act(async () => container.querySelector("button")!.click());
    expect(stop).toHaveBeenCalledWith("running-id");
    expect(navigate).toHaveBeenCalledOnce();
  });
  it("shows snapshotted instructions, full transcript, tools and prior context without duplicated sections", async () => {
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    expect(document.querySelector("dialog")).toBeNull();
    const instructionsButton = container.querySelector<HTMLButtonElement>('[aria-label="Instructions at execution"]')!;
    await act(async () => instructionsButton.click());
    const dialog = document.querySelector("dialog")!;
    expect(dialog.open).toBe(true);
    expect(dialog.textContent).toContain("Saved worker instructions");
    expect(dialog.textContent).not.toContain("Changed since execution");
    expect(dialog.querySelector("pre")?.className).toContain("min-h-0 overflow-y-auto");
    expect(dialog.querySelector("header")?.className).toContain("shrink-0");
    expect(dialog.querySelector("footer")?.className).toContain("shrink-0");
    for (const text of [run.task, "Earlier assignment", "Reasoning evidence", "Full assistant response", "get_account", '"coin":"BTC"', "Complete tool output"]) expect(container.textContent).toContain(text);
    // The assignment and final result duplicate the transcript's first user
    // message and last assistant message, so they no longer get their own sections.
    expect(container.textContent).not.toContain("Complete final result");
    expect([...container.querySelectorAll("h2")].map((heading) => heading.textContent)).not.toContain("Assignment");
    expect([...container.querySelectorAll("h2")].map((heading) => heading.textContent)).not.toContain("Final result");
    expect(container.textContent).not.toContain("Changed since execution");
    expect(container.textContent).not.toContain("Assignment completion is not a filled-trade or protection guarantee.");
    expect(container.textContent).not.toContain("Saved configuration");
    expect(container.textContent).not.toContain("Raw transcript entry");
    await act(async () => dialog.dispatchEvent(new Event("cancel", { cancelable: true })));
    expect(document.querySelector("dialog")).toBeNull();
    await act(async () => instructionsButton.click());
    await act(async () => document.querySelector("dialog button")!.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(document.querySelector("dialog")).toBeNull();
    expect([...container.querySelectorAll("article")].every((article) => article.textContent?.trim())).toBe(true);
    expect(container.querySelectorAll("hr")).toHaveLength(1);
  });
  it("pins the header with an Instructions button and scrolls current and earlier transcript together", async () => {
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    const page = container.firstElementChild!;
    const header = container.querySelector('[aria-label="Run header"]')!;
    const transcript = container.querySelector('[aria-label="Conversation transcript"]')!;
    expect(page.className).toContain("h-[calc(100dvh-4.5rem)]");
    expect(page.className).toContain("overflow-hidden");
    expect(header.className).toContain("shrink-0");
    expect(header.querySelector('[aria-label="Instructions at execution"]')).not.toBeNull();
    expect(transcript.className).toContain("min-h-0 flex-1 overflow-y-auto");
    expect(transcript.textContent).toContain("Earlier assignment");
    expect(transcript.textContent).toContain("Full assistant response");
    expect(transcript.textContent).not.toContain("Instructions at execution");
    expect(page.children).toHaveLength(2);
  });
  it("merges consecutive assistant messages across tool results without crossing user or system messages", async () => {
    const reply = (id: number, text: string) => entry(id, "pi.assistant", [fauxAssistantMessage([{ type: "text", text }], { timestamp: 1 })]);
    const entries = [...initial.entries, reply(5, "After tool response"), reply(6, "Final response"), entry(7, "pi.user", [{ role: "user", content: "Follow-up", timestamp: 1 }]), reply(8, "New turn"), entry(9, "pi.system", [{ role: "system", content: "Updated prompt", timestamp: 1 }]), reply(10, "After system update")];
    await act(async () => root.render(<SubagentRunPage initial={{ ...initial, prior: [], entries }} />));
    const blocks = [...container.querySelectorAll("article")].filter((article) => article.querySelector("h3")?.textContent === "assistant");
    expect(blocks).toHaveLength(3);
    expect(blocks[0]!.querySelectorAll("h3")).toHaveLength(1);
    expect(blocks[0]!.className).toContain("bg-zinc-900");
    expect(blocks[0]!.className).toContain("mr-8");
    const userBubble = [...container.querySelectorAll("article")].find((article) => article.querySelector("h3")?.textContent === "user")!;
    expect(userBubble.className).toContain("bg-violet-950/40");
    expect(userBubble.className).toContain("ml-8");
    const content = blocks[0]!.textContent!;
    for (const text of ["Reasoning evidence", "Full assistant response", "get_account", "Complete tool output", "After tool response", "Final response"]) expect(content).toContain(text);
    expect(content.indexOf("get_account")).toBeLessThan(content.indexOf("After tool response"));
    expect(content.indexOf("After tool response")).toBeLessThan(content.indexOf("Final response"));
    expect(blocks[1]!.textContent).toContain("New turn");
    expect(blocks[2]!.textContent).toContain("After system update");
  });
  it("expands system prompts with named sections in current and earlier context", async () => {
    const system = (id: number, content: string | { type: "text"; text: string }[]) => entry(id, "pi.system", [{ role: "system", content, sections: { instructions: "System section instructions", obsolete: null }, toolsRemoved: [{ name: "old_tool" }], timestamp: 1 }]);
    await act(async () => root.render(<SubagentRunPage initial={{ ...initial, prior: [system(10, "Earlier base prompt")], entries: [system(11, [{ type: "text", text: "Current base prompt" }]), system(12, "")] }} />));
    const systems = [...container.querySelectorAll("details")].filter((details) => details.querySelector(":scope > summary")?.textContent === "System");
    expect(systems).toHaveLength(3);
    for (const details of systems) {
      expect(details.open).toBe(false);
      await act(async () => details.querySelector("summary")!.click());
      expect(details.open).toBe(true);
      expect(details.textContent).toContain("System section instructions");
      expect(details.textContent).toContain("Section removed");
      expect(details.textContent).toContain("Tools removed: old_tool");
    }
    expect(systems[0]!.textContent).toContain("Earlier base prompt");
    expect(systems[1]!.textContent).toContain("Current base prompt");
  });
  it("renders collapsed tools inline and preserves expansion during live updates", async () => {
    const pending = { ...initial, entries: initial.entries.slice(0, 2) };
    await act(async () => root.render(<SubagentRunPage initial={pending} />));
    const transcript = [...container.querySelectorAll("section")].find((section) => section.querySelector("h2")?.textContent === "Conversation transcript")!;
    const tool = [...transcript.querySelectorAll("details")].find((details) => details.querySelector("summary")?.textContent?.includes("get_account"))!;
    expect(tool.open).toBe(false);
    expect(tool.querySelector("summary")?.textContent).toContain("pending");
    expect(transcript.textContent!.indexOf("Full assistant response")).toBeLessThan(transcript.textContent!.indexOf("get_account"));
    await act(async () => { tool.open = true; });
    const completed = { ...initial, tools: [{ id: "call", name: "get_account", status: "done", output: "Bounded result" }] };
    await act(async () => RunSource.instances.at(-1)!.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(completed) })));
    expect(tool.open).toBe(true);
    expect(tool.querySelector("summary")?.textContent).toContain("done");
    expect(tool.textContent).toContain("Bounded result");
    expect(tool.textContent).toContain('"coin":"BTC"');
    expect([...transcript.querySelectorAll("summary")].filter((summary) => summary.textContent?.includes("get_account"))).toHaveLength(1);
    expect(container.textContent).not.toContain("Tool activity");
    expect(tool.textContent).not.toContain("Bounded summary");
    expect(tool.textContent).not.toContain("Open artifacts");
  });
  it("groups live codemode calls and preserves them when script results arrive", async () => {
    const entries = [entry(20, "pi.assistant", [fauxAssistantMessage([{ type: "toolCall", id: "script", name: "codemode", arguments: { code: "await tools.get_account({})" } }], { timestamp: 1 })])];
    const active: RunDetail = { ...initial, prior: [], entries, run: { ...run, status: "running" }, tools: [{ id: "script", name: "codemode", status: "running" }, { id: "script/c1", name: "↳ get_account", status: "running" }] };
    await act(async () => root.render(<SubagentRunPage initial={active} />));
    const panel = container.querySelector('[aria-label="Code execution"]')!;
    expect(panel.textContent).toContain("Running script…");
    const nested = [...panel.querySelectorAll("details")].find((item) => item.textContent?.includes("get_account") && item.querySelector("summary")?.textContent?.includes("get_account"))!;
    expect(nested).toBeDefined();
    await act(async () => { nested.open = true; });
    const completed: RunDetail = { ...active, tools: undefined, entries: [...entries, entry(21, "pi.tool", [{ role: "toolResult", toolCallId: "script", toolName: "codemode", content: [{ type: "text", text: "Analysis complete" }], isError: false, details: { calls: [{ id: 1, name: "get_account", status: "ok", args: "{}" }] }, timestamp: 2 }]), entry(22, "pi.assistant", [fauxAssistantMessage([{ type: "thinking", thinking: "Considering the analysis" }], { timestamp: 3 })])] };
    await act(async () => RunSource.instances.at(-1)!.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(completed) })));
    expect(panel.textContent).toContain("Script completed");
    expect(panel.textContent).toContain("Analysis complete");
    expect(nested.isConnected).toBe(true);
    expect(nested.open).toBe(true);
    expect(nested.querySelector("summary")?.textContent).toContain("ok");
    expect(container.querySelectorAll('[aria-label="Code execution"]')).toHaveLength(1);
    expect([...container.querySelectorAll("summary")].filter((item) => item.textContent?.includes("get_account"))).toHaveLength(1);
    expect(container.textContent).toContain("Considering the analysis");
    expect(panel.textContent).not.toContain("Considering the analysis");
    expect(container.textContent).toContain("Cancel");
  });
  it("labels failed scripts explicitly", async () => {
    await act(async () => root.render(<SubagentRunPage initial={{ ...initial, prior: [], entries: [entry(30, "pi.tool", [{ role: "toolResult", toolCallId: "failed", toolName: "codemode", content: [{ type: "text", text: "Script timed out" }], isError: true, timestamp: 1 }])] }} />));
    const panel = container.querySelector('[aria-label="Code execution"]')!;
    expect(panel.textContent).toContain("Script failed");
    expect(panel.textContent).toContain("Script timed out");
  });
  it("renders a one-line breadcrumb to the sub-agent and live-updates without refresh controls", async () => {
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    const nav = container.querySelector("nav")!;
    expect(nav.getAttribute("aria-label")).toBe("Run breadcrumb");
    const links = [...nav.querySelectorAll("a")].map((link) => ({ href: link.getAttribute("href"), text: link.textContent }));
    expect(links).toEqual([
      { href: "/agents/agent-fixture/sub-agents", text: "← Sub-agents" },
      { href: "/agents/agent-fixture/sub-agents/worker%3Afixture", text: "Research" },
    ]);
    expect(nav.querySelector("h2")?.textContent).toBe("Run large-hex-id");
    const source = RunSource.instances.at(-1)!;
    expect(source.url).toBe("/api/agents/agent-fixture/sub-agents/worker%3Afixture/runs/large-hex-id/events");
    expect([...container.querySelectorAll("button")].some((button) => button.textContent === "Refresh")).toBe(false);
    const live = { ...initial, run: { ...run, status: "running" as const, result: "", finishedAt: null } };
    await act(async () => source.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(live) })));
    expect(container.textContent).toContain("Cancel");
    await act(async () => source.dispatchEvent(new MessageEvent("unavailable", { data: JSON.stringify({ message: "Reconnecting fixture" }) })));
    expect(container.querySelector('[role="status"]')?.textContent).toBe("Reconnecting fixture");
  });
  it("follows live snapshots, pauses when reading above, resumes at the bottom and follows completion", async () => {
    const active = { ...initial, run: { ...run, status: "running" as const, finishedAt: null } };
    await act(async () => root.render(<SubagentRunPage initial={active} />));
    const transcript = container.querySelector('[aria-label="Conversation transcript"]')!;
    Object.defineProperties(transcript, { scrollHeight: { value: 2000, configurable: true }, clientHeight: { value: 600 } });
    const scroll = vi.mocked(Element.prototype.scrollIntoView);
    expect(scroll).toHaveBeenCalledOnce();
    const source = RunSource.instances.at(-1)!;
    const snapshot = async (detail: RunDetail = active) => {
      await act(async () => source.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(detail) })));
    };
    await snapshot();
    expect(scroll).toHaveBeenCalledTimes(2);
    transcript.scrollTop = 400;
    await act(async () => transcript.dispatchEvent(new Event("scroll")));
    await snapshot();
    expect(scroll).toHaveBeenCalledTimes(2);
    transcript.scrollTop = 1400;
    await act(async () => transcript.dispatchEvent(new Event("scroll")));
    await snapshot();
    expect(scroll).toHaveBeenCalledTimes(3);
    await act(async () => source.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(initial) })));
    expect(scroll).toHaveBeenCalledTimes(4);
    await snapshot(initial);
    expect(scroll).toHaveBeenCalledTimes(4);
  });
  it("keeps the server-rendered transcript hidden until positioned before paint", async () => {
    const markup = document.createElement("div");
    markup.innerHTML = renderToString(<SubagentRunPage initial={initial} />);
    expect(markup.querySelector('[aria-label="Conversation transcript"]')?.classList.contains("invisible")).toBe(true);
    expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    expect(Element.prototype.scrollIntoView).toHaveBeenCalledOnce();
    expect(container.querySelector('[aria-label="Conversation transcript"]')?.classList.contains("invisible")).toBe(false);
  });
  it("starts historical runs at the bottom without following subsequent snapshots", async () => {
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    expect(Element.prototype.scrollIntoView).toHaveBeenCalledOnce();
    await act(async () => RunSource.instances.at(-1)!.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(initial) })));
    expect(Element.prototype.scrollIntoView).toHaveBeenCalledOnce();
  });
  it("stops active work through the live stream without manual refreshes", async () => {
    const active = { ...initial, run: { ...run, status: "running" as const, result: "", finishedAt: null } };
    mocks.stopSubagentRun.mockResolvedValue(undefined);
    await act(async () => root.render(<SubagentRunPage initial={active} />));
    const source = RunSource.instances.at(-1)!;
    const cancel = () => [...container.querySelectorAll("button")].find((button) => button.textContent === "Cancel")!;
    await act(async () => cancel().click());
    expect(mocks.stopSubagentRun).toHaveBeenCalledWith({ data: { agentId: run.agentId, runId: run.id } });
    await act(async () => source.dispatchEvent(new MessageEvent("run", { data: JSON.stringify({ ...active, run: { ...active.run, status: "cancelled", result: "Stopped by user" } }) })));
    expect(container.querySelector('[role="img"]')?.getAttribute("aria-label")).toBe("cancelled");
    expect(container.textContent).not.toContain("Cancel");
    mocks.stopSubagentRun.mockRejectedValue(new Error("Unavailable"));
    await act(async () => source.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(active) })));
    await act(async () => cancel().click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Unavailable");
  });
});