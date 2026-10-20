// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ listMemories: vi.fn(), getMemory: vi.fn(), getInsights: vi.fn(), replaceInsights: vi.fn(), readArtifact: vi.fn(), changeArtifact: vi.fn() }));
vi.mock("../web/memories.js", () => mocks);
vi.mock("../web/artifacts.js", () => ({ readArtifact: mocks.readArtifact, changeArtifact: mocks.changeArtifact }));
import { AgentMemoriesPage } from "./agent-memories-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const metadata = { ref: "m1", status: "stale", kind: "research", topic: "trend_setup", instrument: "BTC", interval: "1h", summary: "<img src=x onerror=alert(1)>", createdAt: 1000, evidenceAsOf: 0, validUntil: 1000, producerSubagentId: null, sourceRunId: null, retractedAt: null, retractionReason: null };
const current = { scope: { kind: "agent" }, version: 1, content: "Current guidance" };
const head = { ...current, operation: "replace", reason: "Initial review", createdAt: 1000, userId: null, producerSubagentId: "reviewer", sourceRunId: "run-1", evidenceRefs: ["m1"] };
const data = { current, head, revision: null, history: { items: [{ ...head, content: undefined, evidenceRefs: undefined }], nextBeforeVersion: null } };
describe("Agent memory and insight UI", () => {
  let root: Root, container: HTMLDivElement;
  const dialogMethods = Object.fromEntries(["showModal", "close"].map((key) => [key, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, key)]));
  beforeEach(() => {
    vi.resetAllMocks();
    Object.defineProperties(HTMLDialogElement.prototype, {
      showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
      close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
    });
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    mocks.listMemories.mockResolvedValue({ ok: true, data: { items: [metadata], nextCursor: { createdAt: 1000, ref: "m1" } } });
    mocks.getMemory.mockResolvedValue({ ok: true, data: { ...metadata, content: "<script>bad()</script>", payload: { version: 1, bias: "long" }, links: [{ ref: "m2", relation: "based_on" }], artifactIds: [], executionOrigin: "automatic" } });
    mocks.getInsights.mockResolvedValue({ ok: true, data });
  });
  afterEach(async () => {
    await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks();
    for (const [key, descriptor] of Object.entries(dialogMethods)) {
      if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, key, descriptor);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, key);
    }
  });
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
  it("previews supporting artifacts in a modal with safe content, pagination, attachments and explicit downloads", async () => {
    mocks.getMemory.mockResolvedValue({ ok: true, data: { ...metadata, content: "Report", payload: {}, links: [], artifactIds: ["report"] } });
    const preview = { id: "report", title: "Research report", description: "Evidence", mediaType: "text/plain", content: "<script>untrusted</script>", nextOffset: 100, attachments: [{ id: "chart", title: "Chart", mediaType: "image/png" }] };
    mocks.readArtifact.mockResolvedValue({ ok: true, data: preview });
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} memory="m1" />));
    await click("Supporting artifact report");
    const dialog = () => document.querySelector("dialog")!;
    const modalClick = async (label: string) => act(async () => { [...dialog().querySelectorAll("button")].find((button) => button.textContent === label)!.click(); });
    expect(mocks.readArtifact).toHaveBeenLastCalledWith({ data: { agentId: "agent", id: "report", offset: 0 } });
    expect(dialog().open).toBe(true);
    expect(dialog().textContent).toContain("<script>untrusted</script>");
    expect(dialog().querySelector("script")).toBeNull();
    expect(dialog().querySelector("a")?.getAttribute("href")).toBe("/api/agents/agent/artifacts/report/content");
    mocks.readArtifact.mockResolvedValueOnce({ ok: true, data: { ...preview, content: "More evidence", nextOffset: null } });
    await modalClick("Next text portion");
    expect(mocks.readArtifact).toHaveBeenLastCalledWith({ data: { agentId: "agent", id: "report", offset: 100 } });
    expect(dialog().textContent).toContain("More evidence");
    mocks.readArtifact.mockResolvedValueOnce({ ok: true, data: { ...preview, id: "chart", title: "Chart", mediaType: "image/png", nextOffset: null, attachments: [] } });
    await modalClick("Chart");
    expect(dialog().querySelector("img")?.getAttribute("src")).toBe("/api/agents/agent/artifacts/chart/content?inline=1");
    await modalClick("Back to supporting artifact");
    await act(async () => { dialog().dispatchEvent(new Event("cancel", { cancelable: true })); });
    expect(document.querySelector("dialog")).toBeNull();
    expect(container.textContent).toContain("Report");
  });
  it("manages retention and deletion in the modal while enforcing memory protection", async () => {
    mocks.getMemory.mockResolvedValue({ ok: true, data: { ...metadata, content: "Report", payload: {}, links: [], artifactIds: ["report"] } });
    const preview = { id: "report", title: "Report", description: "", mediaType: "text/plain", content: "Evidence", nextOffset: null, attachments: [], expiresAt: 1000, protectedByMemory: false };
    mocks.readArtifact.mockResolvedValue({ ok: true, data: preview });
    mocks.changeArtifact.mockResolvedValue({ ok: true, data: { updated: true } });
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} memory="m1" />));
    await click("Supporting artifact report");
    const modalButton = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("dialog button")].find((button) => button.textContent === label)!;
    const modalClick = async (label: string) => act(async () => modalButton(label).click());
    mocks.readArtifact.mockResolvedValue({ ok: true, data: { ...preview, expiresAt: null } });
    await modalClick("Keep / Pin");
    expect(mocks.changeArtifact).toHaveBeenLastCalledWith({ data: { agentId: "agent", id: "report", action: "retain" } });
    mocks.readArtifact.mockResolvedValue({ ok: true, data: preview });
    await modalClick("Expire in seven days");
    expect(mocks.changeArtifact).toHaveBeenLastCalledWith({ data: { agentId: "agent", id: "report", action: "expire" } });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    await modalClick("Delete");
    expect(mocks.changeArtifact).toHaveBeenCalledTimes(2);
    confirm.mockReturnValue(true);
    mocks.changeArtifact.mockResolvedValueOnce({ ok: false, error: "Artifact is protected" });
    await modalClick("Delete");
    expect(document.querySelector("dialog [role=alert]")?.textContent).toBe("Artifact is protected");
    mocks.readArtifact.mockResolvedValue({ ok: true, data: { ...preview, protectedByMemory: true, expiresAt: null } });
    await modalClick("Keep / Pin");
    expect(modalButton("Expire in seven days").disabled).toBe(true);
    expect(modalButton("Delete").disabled).toBe(true);
    expect(modalButton("Delete").title).toContain("Evidence referenced by memory");
    await act(async () => modalButton("Close").click());
    mocks.readArtifact.mockResolvedValue({ ok: true, data: preview });
    await click("Supporting artifact report");
    await modalClick("Delete");
    expect(mocks.changeArtifact).toHaveBeenLastCalledWith({ data: { agentId: "agent", id: "report", action: "delete" } });
    expect(document.querySelector("dialog")).toBeNull();
  });
  it("shows artifact read errors in the modal and allows closing it", async () => {
    mocks.getMemory.mockResolvedValue({ ok: true, data: { ...metadata, content: "Report", payload: {}, links: [], artifactIds: ["missing"] } });
    mocks.readArtifact.mockResolvedValue({ ok: false, error: "Artifact not found" });
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} memory="m1" />));
    await click("Supporting artifact missing");
    expect(document.querySelector("dialog [role=alert]")?.textContent).toBe("Artifact not found");
    await act(async () => { document.querySelector<HTMLButtonElement>("dialog button")!.click(); });
    expect(document.querySelector("dialog")).toBeNull();
  });
  it("renders prose-only research without empty data and offers only host-derived status filters", async () => {
    mocks.getMemory.mockResolvedValue({ ok: true, data: { ...metadata, content: "Mixed evidence near support; evaluate before acting.", payload: {}, links: [], artifactIds: [] } });
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} memory="m1" />));
    expect(container.textContent).toContain("Mixed evidence near support; evaluate before acting.");
    expect(container.querySelector('[aria-label="Additional memory data"]')).toBeNull();
    const status = [...container.querySelectorAll("select")].find((select) => select.parentElement?.textContent?.startsWith("Status"))!;
    expect([...status.options].map((option) => option.value)).toEqual(["", "fresh", "stale", "disabled", "retracted", "invalid_evidence", "durable"]);
  });
  it("displays arbitrary research data safely without enforcing signal labels", async () => {
    mocks.getMemory.mockResolvedValue({ ok: true, data: { ...metadata, content: "Report", payload: { bias: "bearish-neutral", custom: { note: "<script>untrusted</script>" } }, links: [], artifactIds: [] } });
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} memory="m1" />));
    const payload = container.querySelector('[aria-label="Additional memory data"]');
    expect(payload?.textContent).toContain("bearish-neutral");
    expect(payload?.textContent).toContain("<script>untrusted</script>");
    expect(container.querySelector("script")).toBeNull();
  });
  it("preserves conflicted drafts and operation UUIDs, and copies historical evidence into a new revision without rewinding", async () => {
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} />));
    await click("Insights");
    expect(container.textContent).toContain("Current v1"); expect(container.textContent).toContain("Initial review");
    const draft = container.querySelector<HTMLTextAreaElement>('[aria-label="Insights document draft"]')!;
    await edit(draft, "Unsubmitted curated draft");
    const reason = [...container.querySelectorAll("input")].find((element) => element.parentElement?.textContent === "Reason")!;
    await edit(reason, "Consolidate");
    mocks.replaceInsights.mockResolvedValue({ ok: false, error: "Insights changed to v2; reread and reconcile" });
    await click("Save new revision"); await click("Save new revision");
    expect(draft.value).toBe("Unsubmitted curated draft"); expect(container.querySelector('[role="alert"]')?.textContent).toContain("preserved");
    const first = mocks.replaceInsights.mock.calls[0]![0].data, second = mocks.replaceInsights.mock.calls[1]![0].data;
    expect(first.requestId).toBe(second.requestId);
    expect(first.input).toMatchObject({ expectedVersion: 1, evidenceRefs: ["m1"] });
    mocks.getInsights.mockResolvedValue({ ok: true, data: { ...data, revision: { ...head, version: 1, content: "Historical guidance" } } });
    await click("v1 · 1970-01-01T00:00:01.000Z · Initial review");
    await click("Copy content to new draft");
    expect(draft.value).toBe("Historical guidance");
    await click("Save new revision");
    expect(mocks.replaceInsights.mock.calls[2]![0].data.input).toMatchObject({ expectedVersion: 1, content: "Historical guidance", evidenceRefs: ["m1"] });
  });
});
