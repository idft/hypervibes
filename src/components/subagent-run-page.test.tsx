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
const definition: SubagentRow = { id: "worker:fixture", agentId: "agent-fixture", kind: "worker", name: "Research", description: "", instructions: "Saved worker instructions", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, archived: false, delegatable: true, persistent: true, conversationId: null, schedule: "", delaySeconds: 30, nextRunAt: null, completion: "save", timeoutSeconds: 900, createdAt: 1, updatedAt: 1 };
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
    for (const text of ["Saved worker instructions", run.task, "Earlier assignment", "Reasoning evidence", "Full assistant response", "Tool call · get_account", '"coin": "BTC"', "Complete tool output"]) expect(container.textContent).toContain(text);
    // The assignment and final result duplicate the transcript's first user
    // message and last assistant message, so they no longer get their own sections.
    expect(container.textContent).not.toContain("Complete final result");
    expect([...container.querySelectorAll("h2")].map((heading) => heading.textContent)).not.toContain("Assignment");
    expect([...container.querySelectorAll("h2")].map((heading) => heading.textContent)).not.toContain("Final result");
    expect(container.textContent).not.toContain("Changed since execution");
    expect(container.querySelectorAll("details").length).toBeGreaterThan(3);
    expect(container.querySelectorAll("hr")).toHaveLength(1);
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