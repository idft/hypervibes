// @vitest-environment jsdom
import { act } from "react";
import { fauxAssistantMessage } from "@earendil-works/pi-ai/providers/faux";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";
import type { EntryRecord } from "@earendil-works/pi-durable";
const mocks = vi.hoisted(() => ({ getSubagentRun: vi.fn(), stopSubagentRun: vi.fn() }));
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
describe("sub-agent run history and details", () => {
  let root: Root, container: HTMLDivElement;
  beforeEach(() => { vi.resetAllMocks(); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); });
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
  it("shows snapshotted instructions, full transcript, tools, prior context and result", async () => {
    await act(async () => root.render(<SubagentRunPage initial={initial} />));
    for (const text of ["Saved worker instructions", run.task, "Earlier assignment", "Reasoning evidence", "Full assistant response", "Tool call · get_account", '"coin": "BTC"', "Complete tool output", run.result]) expect(container.textContent).toContain(text);
    expect(container.textContent).not.toContain("Changed since execution");
    expect(container.querySelectorAll("details").length).toBeGreaterThan(3);
  });
  it("refreshes after stopping active work without hiding errors", async () => {
    const active = { ...initial, run: { ...run, status: "running" as const, result: "", finishedAt: null } };
    mocks.stopSubagentRun.mockResolvedValue({ ok: true });
    mocks.getSubagentRun.mockResolvedValue({ ok: true, data: { ...active, run: { ...active.run, status: "cancelled", result: "Stopped by user" } } });
    await act(async () => root.render(<SubagentRunPage initial={active} />));
    await act(async () => [...container.querySelectorAll("button")].find((button) => button.textContent === "Stop assignment")!.click());
    expect(mocks.stopSubagentRun).toHaveBeenCalledWith({ data: { agentId: run.agentId, runId: run.id } });
    expect(container.textContent).toContain("Stopped by user");
    expect(container.textContent).not.toContain("Stop assignment");
    mocks.getSubagentRun.mockRejectedValue(new Error("Unavailable"));
    await act(async () => container.querySelector("button")!.click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Unavailable");
    expect(container.textContent).toContain("Stopped by user");
  });
});
