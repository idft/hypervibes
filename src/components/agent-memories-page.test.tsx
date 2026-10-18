// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ listMemories: vi.fn(), getMemory: vi.fn(), getLearnings: vi.fn(), replaceLearnings: vi.fn() }));
vi.mock("../web/memories.js", () => mocks);
import { AgentMemoriesPage } from "./agent-memories-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const metadata = { ref: "m1", status: "stale", kind: "research", topic: "trend_setup", instrument: "BTC", interval: "1h", summary: "<img src=x onerror=alert(1)>", createdAt: 1000, evidenceAsOf: 0, validUntil: 1000, producerSubagentId: null, sourceRunId: null, retractedAt: null, retractionReason: null };
const current = { scope: { kind: "agent" }, version: 1, content: "Current guidance" };
const head = { ...current, operation: "replace", reason: "Initial review", createdAt: 1000, userId: null, producerSubagentId: "reviewer", sourceRunId: "run-1", evidenceRefs: ["m1"] };
const data = { current, head, revision: null, history: { items: [{ ...head, content: undefined, evidenceRefs: undefined }], nextBeforeVersion: null } };
describe("Agent memory and learning UI", () => {
  let root: Root, container: HTMLDivElement;
  beforeEach(() => {
    vi.resetAllMocks(); container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    mocks.listMemories.mockResolvedValue({ ok: true, data: { items: [metadata], nextCursor: { createdAt: 1000, ref: "m1" } } });
    mocks.getMemory.mockResolvedValue({ ok: true, data: { ...metadata, content: "<script>bad()</script>", payload: { version: 1, bias: "long" }, links: [{ ref: "m2", relation: "based_on" }], artifactIds: [], executionOrigin: "automatic" } });
    mocks.getLearnings.mockResolvedValue({ ok: true, data });
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); });
  const click = async (name: string) => act(async () => { [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === name)!.click(); });
  async function edit(element: HTMLInputElement | HTMLTextAreaElement, value: string) {
    await act(async () => { Object.getOwnPropertyDescriptor(element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(element, value); element.dispatchEvent(new Event("input", { bubbles: true })); });
  }
  it("renders safe text, exact deep links and relationship lookups with exclusive metadata pagination", async () => {
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} memory="m1" />));
    expect(mocks.getMemory).toHaveBeenCalledWith({ data: { agentId: "agent", ref: "m1" } });
    expect(container.textContent).toContain("<img src=x onerror=alert(1)>"); expect(container.textContent).toContain("<script>bad()</script>");
    expect(container.querySelector("img")).toBeNull(); expect(container.querySelector("script")).toBeNull();
    expect(container.textContent).toContain("stale");
    await click("based_on m2");
    expect(mocks.getMemory).toHaveBeenLastCalledWith({ data: { agentId: "agent", ref: "m2" } });
    await click("Next page");
    expect(mocks.listMemories).toHaveBeenLastCalledWith({ data: { agentId: "agent", filters: { cursor: { createdAt: 1000, ref: "m1" } } } });
    expect([...container.querySelectorAll("button")].every((button) => button.className.includes("cursor-pointer"))).toBe(true);
  });
  it("preserves conflicted drafts and operation UUIDs, and copies historical evidence into a new revision without rewinding", async () => {
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} />));
    await click("Accumulated learnings");
    expect(container.textContent).toContain("Current v1"); expect(container.textContent).toContain("Initial review");
    const draft = container.querySelector<HTMLTextAreaElement>('[aria-label="Learning document draft"]')!;
    await edit(draft, "Unsubmitted curated draft");
    const reason = [...container.querySelectorAll("input")].find((element) => element.parentElement?.textContent === "Reason")!;
    await edit(reason, "Consolidate");
    mocks.replaceLearnings.mockResolvedValue({ ok: false, error: "Learnings changed to v2; reread and reconcile" });
    await click("Save new revision"); await click("Save new revision");
    expect(draft.value).toBe("Unsubmitted curated draft"); expect(container.querySelector('[role="alert"]')?.textContent).toContain("preserved");
    const first = mocks.replaceLearnings.mock.calls[0]![0].data, second = mocks.replaceLearnings.mock.calls[1]![0].data;
    expect(first.requestId).toBe(second.requestId);
    expect(first.input).toMatchObject({ expectedVersion: 1, evidenceRefs: ["m1"] });
    mocks.getLearnings.mockResolvedValue({ ok: true, data: { ...data, revision: { ...head, version: 1, content: "Historical guidance" } } });
    await click("v1 · 1970-01-01T00:00:01.000Z · Initial review");
    await click("Copy content to new draft");
    expect(draft.value).toBe("Historical guidance");
    await click("Save new revision");
    expect(mocks.replaceLearnings.mock.calls[2]![0].data.input).toMatchObject({ expectedVersion: 1, content: "Historical guidance", evidenceRefs: ["m1"] });
  });
});
