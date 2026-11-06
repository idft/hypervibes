// @vitest-environment jsdom
import { act } from "react";
import type { QueryClient } from "@tanstack/react-query";
import { createTestQueryClient, withTestQueryClient } from "../test/query.js";
import { fauxAssistantMessage } from "@earendil-works/pi-ai/providers/faux";
import { createRoot, type Root } from "react-dom/client";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RunDetail } from "../agents/subagent-service.js";
import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";
import type { EntryRecord } from "@earendil-works/pi-durable";
const mocks = vi.hoisted(() => ({ stopSubagentRun: vi.fn(), sendSubagentRunFollowup: vi.fn() }));
vi.mock("../web/subagents.js", () => mocks);
import { SubagentRunHistory } from "./subagent-run-history.js";
import { SubagentRunPage as SubagentRunPageComponent } from "./subagent-run-page.js";
let client: QueryClient;
const SubagentRunPage = withTestQueryClient(SubagentRunPageComponent, () => client);
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
  beforeEach(() => { vi.resetAllMocks(); client = createTestQueryClient(); RunSource.instances = []; vi.stubGlobal("EventSource", RunSource); container = document.createElement("div"); container.className = "app-shell"; Element.prototype.scrollIntoView = vi.fn(); document.body.append(container); root = createRoot(container); });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
  async function typeFollowup(text: string) {
    const textarea = container.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, text);
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
  }
  async function submitFollowup() {
    await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  }
  async function snapshot(detail: RunDetail) {
    await act(async () => RunSource.instances.at(-1)!.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(detail) })));
  }
  it.each(["completed", "failed", "cancelled"] as const)("adds a read-only discussion and accessible footer for %s runs", async (status) => {
    await act(async () => root.render(<SubagentRunPage initial={{ ...initial, run: { ...run, status } }} />));
    const transcript = container.querySelector('[aria-label="Conversation transcript"]')!;
    const discussion = transcript.querySelector('[aria-label="Follow-up discussion"]')!;
    const form = container.querySelector<HTMLFormElement>('form[aria-label="Run follow-up"]')!;
    expect(discussion.querySelector("h2")?.textContent).toBe("Follow-up discussion");
    expect(discussion.querySelector("header p")).toBeNull();
    expect(discussion.textContent).toContain("Read-only");
    expect(transcript.textContent).toContain("Full assistant response");
    expect(transcript.textContent).toContain("Earlier assignment");
    expect(discussion.textContent).not.toContain("Full assistant response");
    expect(discussion.textContent).not.toContain("Earlier assignment");
    expect(transcript.contains(form)).toBe(false);
    expect(form.parentElement).toBe(transcript.parentElement);
    expect(form.className).toContain("shrink-0");
    expect(form.className).not.toMatch(/(?:sm|md):(?:block|flex)/);
    expect(form.querySelector("label")?.textContent).toBe("Follow-up message");
    expect(form.querySelector("label")?.htmlFor).toBe(form.querySelector("textarea")?.id);
    expect(form.querySelector("textarea")?.maxLength).toBe(16000);
    expect(form.querySelector("textarea")?.className).toContain("min-w-0");
    expect(form.querySelector("button")?.className).toContain("cursor-pointer");
    expect(form.querySelector("button")?.disabled).toBe(true);
    expect(form.querySelector("textarea")?.disabled).toBe(false);
  });
  it.each(["queued", "running", "skipped"] as const)("does not offer a discussion for %s runs", async (status) => {
    await act(async () => root.render(<SubagentRunPage initial={{ ...initial, run: { ...run, status } }} />));
    expect(container.querySelector('[aria-label="Follow-up discussion"]')).toBeNull();
    expect(container.querySelector("form")).toBeNull();
    expect(container.querySelector("textarea")).toBeNull();
  });
  it("starts a discussion when a running run finishes through the same stream", async () => {
    await act(async () => root.render(<SubagentRunPage initial={{ ...initial, run: { ...run, status: "running" } }} />));
    expect(container.querySelector("textarea")).toBeNull();
    await snapshot(initial);
    expect(container.querySelector("textarea")).not.toBeNull();
    expect(RunSource.instances).toHaveLength(1);
  });
  it("submits once, clears only the sent draft, and waits for the existing stream", async () => {
    let finish!: () => void;
    mocks.sendSubagentRunFollowup.mockImplementation(() => new Promise<void>((resolve) => { finish = resolve; }));
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    await typeFollowup("Why did this run stop?");
    const form = container.querySelector("form")!;
    await act(async () => {
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    expect(mocks.sendSubagentRunFollowup).toHaveBeenCalledOnce();
    expect(mocks.sendSubagentRunFollowup).toHaveBeenCalledWith({ data: { agentId: run.agentId, id: definition.id, runId: run.id, message: "Why did this run stop?", requestId: expect.any(String) } });
    expect(container.querySelector('button[type="submit"]')?.textContent).toBe("Sending…");
    expect(container.querySelector<HTMLButtonElement>('button[type="submit"]')?.disabled).toBe(true);
    expect(container.querySelector("textarea")?.value).toBe("Why did this run stop?");
    await typeFollowup("Another question");
    await act(async () => finish());
    expect(container.querySelector("textarea")?.value).toBe("Another question");
    expect(container.querySelector('[aria-label="Follow-up discussion"]')?.textContent).not.toContain("Why did this run stop?");
    mocks.sendSubagentRunFollowup.mockResolvedValue(undefined);
    await submitFollowup();
    expect(container.querySelector("textarea")?.value).toBe("");
    expect(document.activeElement).toBe(container.querySelector("textarea"));
    expect(RunSource.instances).toHaveLength(1);
  });
  it("preserves failed drafts and request IDs for retries, but gives changed messages new IDs", async () => {
    mocks.sendSubagentRunFollowup.mockRejectedValue(new Error("Connection lost"));
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    await typeFollowup("Explain the result");
    await submitFollowup();
    const first = mocks.sendSubagentRunFollowup.mock.calls[0]![0] as { data: { requestId: string } };
    expect(container.querySelector("textarea")?.value).toBe("Explain the result");
    expect(container.querySelector('[role="alert"]')?.textContent).toBe("Connection lost");
    expect(container.querySelector("textarea")?.getAttribute("aria-describedby")).toBe(container.querySelector('[role="alert"]')?.id);
    await submitFollowup();
    expect(mocks.sendSubagentRunFollowup.mock.calls[1]![0]).toEqual(first);
    await typeFollowup("Explain the failure instead");
    await submitFollowup();
    expect(mocks.sendSubagentRunFollowup.mock.calls[2]![0].data.requestId).not.toBe(first.data.requestId);
    const retry = mocks.sendSubagentRunFollowup.mock.calls[2]![0];
    mocks.sendSubagentRunFollowup.mockResolvedValue(undefined);
    await submitFollowup();
    expect(mocks.sendSubagentRunFollowup.mock.calls[3]![0]).toEqual(retry);
    expect(container.querySelector("textarea")?.value).toBe("");
    expect(container.querySelector('[role="alert"]')).toBeNull();
    await typeFollowup("Explain the failure instead");
    await submitFollowup();
    expect(mocks.sendSubagentRunFollowup.mock.calls[4]![0].data.requestId).not.toBe(retry.data.requestId);
  });
  it("rejects blank messages and supports Enter, Shift+Enter, and composition", async () => {
    mocks.sendSubagentRunFollowup.mockResolvedValue(undefined);
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    await typeFollowup("   "); await submitFollowup();
    expect(mocks.sendSubagentRunFollowup).not.toHaveBeenCalled();
    await typeFollowup("Explain this run");
    const textarea = container.querySelector("textarea")!;
    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true, cancelable: true })));
    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true, cancelable: true })));
    expect(mocks.sendSubagentRunFollowup).not.toHaveBeenCalled();
    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(mocks.sendSubagentRunFollowup).toHaveBeenCalledOnce();
  });
  it("renders branch history, partial replies, busy state, and failures from run snapshots", async () => {
    const discussion = { conversationId: 8, entries: [entry(51, "pi.user", [{ role: "user", content: "What happened?", timestamp: 1 }])], busy: true, partial: "Checking the result" };
    await act(async () => root.render(<SubagentRunPage initial={{ ...initial, discussion }} />));
    const branch = container.querySelector('[aria-label="Follow-up discussion"]')!;
    expect(branch.textContent).toContain("What happened?");
    expect(branch.querySelector('[role="status"]')?.textContent).toBe("Checking the result");
    expect(branch.querySelector('[role="status"]')?.closest("article")?.className).toContain("bg-zinc-900");
    expect(container.querySelector("textarea")?.disabled).toBe(true);
    await submitFollowup();
    expect(mocks.sendSubagentRunFollowup).not.toHaveBeenCalled();
    await snapshot({ ...initial, discussion: { ...discussion, partial: "" } });
    expect(branch.querySelector('[role="status"]')?.textContent).toBe("Thinking…");
    const response = entry(52, "pi.assistant", [fauxAssistantMessage("The run finished normally.")]);
    await snapshot({ ...initial, discussion: { ...discussion, entries: [...discussion.entries, response], busy: false, partial: "", error: "Provider disconnected" } });
    expect(branch.textContent).toContain("The run finished normally.");
    expect(client.getQueryData<RunDetail>(["subagents", run.agentId, "run", run.subagentId, run.id])?.discussion?.entries).toHaveLength(2);
    expect(branch.textContent).not.toContain("Thinking…");
    expect(container.querySelector('[role="alert"]')?.textContent).toBe("Provider disconnected");
    expect(container.querySelector("textarea")?.disabled).toBe(false);
    expect(container.textContent?.match(/Full assistant response/g)).toHaveLength(1);
    expect(RunSource.instances).toHaveLength(1);
  });
  it("shows unavailable reasons and disables submissions without discarding a draft or branch history", async () => {
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    await typeFollowup("Explain the result");
    const unavailable = { ...initial, discussionUnavailable: "The original conversation is unavailable.", discussion: { conversationId: 8, entries: [entry(51, "pi.user", [{ role: "user", content: "Saved question", timestamp: 1 }])], busy: false, partial: "" } };
    await snapshot(unavailable);
    const form = container.querySelector("form")!;
    expect(form.querySelector('[role="status"]')?.textContent).toBe(unavailable.discussionUnavailable);
    expect(form.querySelector("textarea")?.disabled).toBe(true);
    expect(form.querySelector("textarea")?.value).toBe("Explain the result");
    expect(form.querySelector("button")?.disabled).toBe(true);
    expect(container.querySelector('[aria-label="Follow-up discussion"]')?.textContent).toContain("Saved question");
    await submitFollowup();
    expect(mocks.sendSubagentRunFollowup).not.toHaveBeenCalled();
  });
  it("follows discussion updates, pauses above the bottom, and does not follow identical historical snapshots", async () => {
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    const transcript = container.querySelector('[aria-label="Conversation transcript"]')!;
    Object.defineProperties(transcript, { scrollHeight: { value: 2000 }, clientHeight: { value: 600 } });
    const scroll = vi.mocked(Element.prototype.scrollIntoView);
    const discussion = { conversationId: 8, entries: [], busy: true, partial: "First" };
    await snapshot({ ...initial, discussion });
    expect(scroll).toHaveBeenCalledTimes(2);
    await snapshot({ ...initial, discussion });
    expect(scroll).toHaveBeenCalledTimes(2);
    transcript.scrollTop = 400;
    await act(async () => transcript.dispatchEvent(new Event("scroll")));
    await snapshot({ ...initial, discussion: { ...discussion, partial: "Second" } });
    expect(scroll).toHaveBeenCalledTimes(2);
    transcript.scrollTop = 1400;
    await act(async () => transcript.dispatchEvent(new Event("scroll")));
    await snapshot({ ...initial, discussion: { ...discussion, partial: "Final", busy: false } });
    expect(scroll).toHaveBeenCalledTimes(3);
    await snapshot({ ...initial, discussion: { ...discussion, partial: "", busy: false, entries: [entry(51, "pi.assistant", [fauxAssistantMessage("Final")])] } });
    expect(scroll).toHaveBeenCalledTimes(4);
  });
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
  it.each([true, false])("copies the run ID with honest feedback (success: %s)", async (success) => {
    const writeText = success ? vi.fn().mockResolvedValue(undefined) : vi.fn().mockRejectedValue(new Error("Clipboard denied"));
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    const copy = container.querySelector<HTMLButtonElement>('[aria-label="Copy run ID"]')!;
    expect(copy.querySelector("svg")).not.toBeNull();
    expect(copy.textContent).toBe("");
    expect(copy.className).not.toContain("border");
    expect(copy.className).toContain("cursor-pointer");
    await act(async () => copy.click());
    expect(writeText).toHaveBeenCalledWith(run.id);
    expect(container.querySelector('[aria-label="Run breadcrumb"] [role="status"]')?.textContent).toBe(success ? "Copied" : "Unable to copy");
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
  it("shows the assignment failure reason even when no transcript exists", async () => {
    const result = "Assignment timed out. Existing exchange orders were not cancelled.";
    await act(async () => root.render(<SubagentRunPage initial={{ ...initial, entries: [], run: { ...run, status: "failed", result } }} />));
    const errors = container.querySelector('[aria-label="Run error"]')!;
    expect(errors.querySelector("h2")?.textContent).toBe("Run error");
    expect(errors.className).toContain("max-h-[40%]");
    expect(errors.className).toContain("overflow-y-auto");
    expect(errors.parentElement?.className).toContain("min-h-0 flex-1");
    expect(errors.parentElement).toBe(container.querySelector('[aria-label="Conversation transcript"]')?.parentElement);
    expect(errors.textContent).toContain(result);
    expect(errors.textContent).not.toContain("Earlier assignment");
    expect(container.querySelector('[aria-label="Conversation transcript"]')?.textContent).toContain("No transcript entries yet.");
  });
  it.each(["completed", "running", "queued", "cancelled", "skipped"] as const)("does not show a run error for %s runs with tool or provider failures", async (status) => {
    const failed = entry(31, "pi.tool-result", [{ role: "toolResult", toolCallId: "script", toolName: "codemode", content: [{ type: "text", text: 'Unknown indicator input: "length"' }], isError: true, details: { calls: [{ id: 1, name: "indicator_run", status: "error", error: 'Unknown indicator input: "length"' }] }, timestamp: 1 }]);
    const provider = entry(32, "pi.assistant", [fauxAssistantMessage("", { stopReason: "error", errorMessage: "Provider rate limit reached" })]);
    await act(async () => root.render(<SubagentRunPage initial={{ ...initial, entries: [failed, provider], run: { ...run, status } }} />));
    expect(container.querySelector('[aria-label="Run error"]')).toBeNull();
  });
  it("shows only the run failure reason through the existing live stream", async () => {
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    expect(container.querySelector('[aria-label="Run error"]')).toBeNull();
    const failure = entry(40, "pi.tool-result", [{ role: "toolResult", toolCallId: "script", toolName: "codemode", content: [{ type: "text", text: "Script error details" }], isError: true, timestamp: 1 }]);
    const result = "Assignment timed out. Existing exchange orders were not cancelled.";
    const updated = { ...initial, entries: [...initial.entries, failure], run: { ...run, status: "failed", result } };
    const source = RunSource.instances.at(-1)!;
    await act(async () => source.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(updated) })));
    const errors = container.querySelector('[aria-label="Run error"]')!;
    expect(errors.textContent).toContain(result);
    expect(errors.textContent).not.toContain("Script error details");
    expect(errors.textContent).not.toContain("codemode");
    await act(async () => source.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(initial) })));
    expect(container.querySelector('[aria-label="Run error"]')).toBeNull();
  });
  it("shows a fallback for failed runs without a recorded reason", async () => {
    await act(async () => root.render(<SubagentRunPage initial={{ ...initial, run: { ...run, status: "failed", result: "" } }} />));
    expect(container.querySelector('[aria-label="Run error"]')?.textContent).toContain("No failure details recorded.");
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