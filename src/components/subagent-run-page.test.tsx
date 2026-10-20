// @vitest-environment jsdom
import { act } from "react";
import { fauxAssistantMessage } from "@earendil-works/pi-ai/providers/faux";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";
import type { EntryRecord } from "@earendil-works/pi-durable";
const mocks = vi.hoisted(() => ({ stopSubagentRun: vi.fn() }));
vi.mock("../web/subagents.js", () => mocks);
import { SubagentRunHistory } from "./subagent-run-history.js";
import { SubagentRunPage } from "./subagent-run-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const definition: SubagentRow = { id: "worker:fixture", agentId: "agent-fixture", kind: "worker", name: "Research", description: "", instructions: "Saved worker instructions", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], templateOrigin: null, enabled: true, archived: false, delegatable: true, persistent: true, conversationId: null, schedule: "", delaySeconds: 30, nextRunAt: null, completion: "save", timeoutSeconds: 900, createdAt: 1, updatedAt: 1 };
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
  beforeEach(() => { vi.resetAllMocks(); RunSource.instances = []; vi.stubGlobal("EventSource", RunSource); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
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
    for (const text of ["Saved worker instructions", run.task, "Earlier assignment", "Reasoning evidence", "Full assistant response", "get_account", '"coin":"BTC"', "Complete tool output"]) expect(container.textContent).toContain(text);
    // The assignment and final result duplicate the transcript's first user
    // message and last assistant message, so they no longer get their own sections.
    expect(container.textContent).not.toContain("Complete final result");
    expect([...container.querySelectorAll("h2")].map((heading) => heading.textContent)).not.toContain("Assignment");
    expect([...container.querySelectorAll("h2")].map((heading) => heading.textContent)).not.toContain("Final result");
    expect(container.textContent).not.toContain("Changed since execution");
    expect(container.textContent).not.toContain("Assignment completion is not a filled-trade or protection guarantee.");
    expect(container.textContent).not.toContain("Saved configuration");
    expect(container.textContent).not.toContain("Raw transcript entry");
    const instructions = [...container.querySelectorAll("details")].find((details) => details.querySelector("summary")?.textContent === "Instructions at execution")!;
    expect(instructions.open).toBe(false);
    expect(instructions.textContent).toContain("Saved worker instructions");
    await act(async () => instructions.querySelector("summary")!.click());
    expect(instructions.open).toBe(true);
    expect([...container.querySelectorAll("article")].every((article) => article.textContent?.trim())).toBe(true);
    expect(container.querySelectorAll("hr")).toHaveLength(1);
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
    expect(container.textContent).toContain("Stop assignment");
    await act(async () => source.dispatchEvent(new MessageEvent("unavailable", { data: JSON.stringify({ message: "Reconnecting fixture" }) })));
    expect(container.querySelector('[role="status"]')?.textContent).toBe("Reconnecting fixture");
  });
  it("stops active work through the live stream without manual refreshes", async () => {
    const active = { ...initial, run: { ...run, status: "running" as const, result: "", finishedAt: null } };
    mocks.stopSubagentRun.mockResolvedValue({ ok: true });
    await act(async () => root.render(<SubagentRunPage initial={active} />));
    const source = RunSource.instances.at(-1)!;
    await act(async () => container.querySelector("button")!.click());
    expect(mocks.stopSubagentRun).toHaveBeenCalledWith({ data: { agentId: run.agentId, runId: run.id } });
    await act(async () => source.dispatchEvent(new MessageEvent("run", { data: JSON.stringify({ ...active, run: { ...active.run, status: "cancelled", result: "Stopped by user" } }) })));
    expect(container.querySelector('[role="img"]')?.getAttribute("aria-label")).toBe("cancelled");
    expect(container.textContent).not.toContain("Stop assignment");
    mocks.stopSubagentRun.mockResolvedValue({ ok: false, error: "Unavailable" });
    await act(async () => source.dispatchEvent(new MessageEvent("run", { data: JSON.stringify(active) })));
    await act(async () => container.querySelector("button")!.click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Unavailable");
  });
});