// @vitest-environment jsdom
import { act, type ReactNode } from "react";
import type { MemoryDetail } from "../memory/memory-contracts.js";
import type { SubagentRow } from "../db/subagent-schema.js";
import { createRoot, type Root } from "react-dom/client";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ listMemories: vi.fn(), getInsights: vi.fn(), replaceInsights: vi.fn(), readArtifact: vi.fn(), changeArtifact: vi.fn() }));
vi.mock("../web/memories.js", () => mocks);
vi.mock("@tanstack/react-router", () => ({ Link: ({ to, params, children, className }: { to: string; params: Record<string, string>; children: ReactNode; className?: string }) => <a className={className} href={to.replace(/\$(\w+)/g, (_match, key: string) => encodeURIComponent(params[key]!))}>{children}</a> }));
vi.mock("../web/artifacts.js", () => ({ readArtifact: mocks.readArtifact, changeArtifact: mocks.changeArtifact }));
import { AgentMemoriesPage } from "./agent-memories-page.js";
import { AgentMemoryDetailPage } from "./agent-memory-detail-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const metadata = { ref: "m1", status: "stale", kind: "research", topic: "trend_setup", instrument: "BTC", interval: "1h", summary: "<img src=x onerror=alert(1)>", createdAt: 1000, evidenceAsOf: 0, validUntil: 1000, producerSubagentId: null, sourceRunId: null, retractedAt: null, retractionReason: null };
const current = { scope: { kind: "agent" }, version: 1, content: "Current guidance" };
const head = { ...current, operation: "replace", reason: "Initial review", createdAt: 1000, userId: null, producerSubagentId: "reviewer", sourceRunId: "run-1", evidenceRefs: ["m1"] };
const data = { current, head, revision: null, history: { items: [{ ...head, content: undefined, evidenceRefs: undefined }], nextBeforeVersion: null } };
describe("Agent memory and insight UI", () => {
  let root: Root, container: HTMLDivElement, detail: MemoryDetail;
  const clipboardDescriptor = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  const dialogMethods = Object.fromEntries(["showModal", "close"].map((key) => [key, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, key)]));
  beforeEach(() => {
    vi.resetAllMocks();
    Object.defineProperties(HTMLDialogElement.prototype, {
      showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
      close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
    });
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    mocks.listMemories.mockResolvedValue({ ok: true, data: { items: [metadata], nextCursor: { createdAt: 1000, ref: "m1" } } });
    detail = { ...metadata, kind: "research", conversationId: null, toolTaskId: null, content: "<script>bad()</script>", payload: { version: 1, bias: "long" }, links: [{ ref: "m2", relation: "based_on" }], artifactIds: [] };
    mocks.getInsights.mockResolvedValue({ ok: true, data });
  });
  afterEach(async () => {
    await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks();
    if (clipboardDescriptor) Object.defineProperty(navigator, "clipboard", clipboardDescriptor);
    else Reflect.deleteProperty(navigator, "clipboard");
    for (const [key, descriptor] of Object.entries(dialogMethods)) {
      if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, key, descriptor);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, key);
    }
  });
  const click = async (name: string) => act(async () => { [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === name)!.click(); });
  async function edit(element: HTMLInputElement | HTMLTextAreaElement, value: string) {
    await act(async () => { Object.getOwnPropertyDescriptor(element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(element, value); element.dispatchEvent(new Event("input", { bubbles: true })); });
  }
  const renderDetail = async () => {
    await act(async () => root.render(<AgentMemoryDetailPage agentId="agent" memory={detail} />));
  };
  it("renders safe memory detail with back and relationship links, without the index", async () => {
    await renderDetail();
    expect(container.querySelector("h1")?.textContent).toBe("Memory m1");
    expect(container.querySelector("table")).toBeNull();
    expect(container.querySelector("a")?.getAttribute("href")).toBe("/agents/agent/memories");
    expect(container.textContent).toContain("<img src=x onerror=alert(1)>"); expect(container.textContent).toContain("<script>bad()</script>");
    expect(container.querySelector('img[src="x"]')).toBeNull(); expect(container.querySelector("script")).toBeNull();
    expect(container.textContent).toContain("stale");
    expect([...container.querySelectorAll("a")].find((link) => link.textContent === "Based on: m2")?.getAttribute("href")).toBe("/agents/agent/memories/m2");
    expect(mocks.listMemories).not.toHaveBeenCalled();
    expect([...container.querySelectorAll("button")].every((button) => button.className.includes("cursor-pointer"))).toBe(true);
  });
  it("places a compact copy control beside the title and metadata above the contents", async () => {
    detail = { ...detail, producerSubagentId: "worker", sourceRunId: "run-1" };
    await renderDetail();
    const heading = container.querySelector("h1")!;
    const copy = container.querySelector<HTMLButtonElement>('button[aria-label="Copy memory reference"]')!;
    const breadcrumb = container.querySelector('nav[aria-label="Memory breadcrumb"]')!;
    expect(breadcrumb.contains(heading)).toBe(true);
    expect(heading.getAttribute("aria-current")).toBe("page");
    expect(breadcrumb.querySelector("a")?.textContent).toBe("← Memories");
    expect(breadcrumb.querySelector("a")?.getAttribute("href")).toBe("/agents/agent/memories");
    expect(copy.parentElement).toBe(heading.parentElement);
    expect(copy.nextElementSibling?.textContent).toBe("stale");
    expect(copy.nextElementSibling?.className).toContain("text-yellow-300");
    expect(container.querySelector("header > p")).toBeNull();
    expect(copy.querySelector("svg")).not.toBeNull();
    expect(copy.textContent).toBe("");
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    await act(async () => copy.click());
    expect(writeText).toHaveBeenCalledWith("m1");
    const contents = container.querySelector('[aria-label="Memory contents"]')!;
    const source = [...container.querySelectorAll("a")].find((link) => link.textContent === "Source run")!;
    expect(source.getAttribute("href")).toBe("/agents/agent/sub-agents/worker/runs/run-1");
    const stats = container.querySelector('dl[aria-label="Memory stats"]')!;
    expect([...stats.querySelectorAll("dt")].map((label) => label.textContent)).toEqual(["Kind", "Instrument", "Topic", "Interval", "Created", "Evidence boundary", "Valid until", "Producer", "Source", "Related memories"]);
    expect(stats.querySelector("time")?.dateTime).toBe("1970-01-01T00:00:01.000Z");
    const created = stats.querySelector<HTMLButtonElement>('button[aria-label="Show UTC creation timestamp"]')!;
    expect(created.textContent).toContain("ago");
    await act(async () => created.click());
    expect(created.textContent).toBe("1970-01-01 00:00:01 UTC");
    const expiry = stats.querySelector<HTMLButtonElement>('button[aria-label="Show UTC expiry timestamp"]')!;
    expect(expiry.textContent).toContain("ago");
    await act(async () => expiry.click());
    expect(expiry.textContent).toBe("1970-01-01 00:00:01 UTC");
    await act(async () => expiry.click());
    expect(expiry.textContent).toContain("ago");
    expect([...stats.querySelectorAll("a")].find((anchor) => anchor.textContent === "worker")?.getAttribute("href")).toBe("/agents/agent/sub-agents/worker");
    for (const value of ["research", "trend_setup"]) {
      const badge = [...stats.querySelectorAll("span")].find((span) => span.textContent === value)!;
      expect(badge.className).toContain("rounded-full");
      expect(badge.className).toContain("text-zinc-300");
    }
    expect(stats.querySelector("img")?.getAttribute("src")).toBe("/currency/BTC.svg");
    expect(stats.contains(source)).toBe(true);
    for (const element of [source, stats]) {
      expect(element.compareDocumentPosition(contents) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
    }
    expect(contents.textContent).toContain(metadata.summary);
    expect(contents.textContent).toContain(detail.content);
  });
  it("shows explicit stats fallbacks for agent-wide memories without market expiry", async () => {
    detail = { ...detail, instrument: null, interval: null, evidenceAsOf: null, validUntil: null, links: [] };
    await renderDetail();
    const stats = container.querySelector('dl[aria-label="Memory stats"]')!;
    const values = Object.fromEntries([...stats.querySelectorAll("dt")].map((label) => [label.textContent, label.nextElementSibling?.textContent]));
    expect(values).toMatchObject({ Instrument: "Agent-wide", Interval: "—", "Evidence boundary": "None", "Valid until": "No market expiry", Source: "Root authored", Producer: "Root agent" });
    expect(values).not.toHaveProperty("Related memories");
  });
  it("uses readable labels for all related memory relations", async () => {
    detail = { ...detail, links: [{ ref: "m2", relation: "based_on" }, { ref: "m3", relation: "corrects" }, { ref: "m4", relation: "supersedes" }, { ref: "m5", relation: "reviews" }] };
    await renderDetail();
    const related = [...container.querySelectorAll("dt")].find((label) => label.textContent === "Related memories")!.nextElementSibling!;
    expect([...related.querySelectorAll("a")].map((anchor) => anchor.textContent)).toEqual(["Based on: m2", "Corrects: m3", "Supersedes: m4", "Reviews: m5"]);
  });
  it("renders preloaded memories immediately, without an empty flash or duplicate fetch", async () => {
    const initialMemories = { items: [{ ...detail }], nextCursor: { createdAt: 1000, ref: "m1" } };
    const page = <AgentMemoriesPage agentId="agent" workers={[]} initialMemories={initialMemories} />;
    const html = renderToString(page);
    expect(html).toContain("m1");
    expect(html).toContain("&lt;img src=x onerror=alert(1)&gt;");
    expect(html).not.toContain("No memories found");
    await act(async () => root.render(page));
    expect(container.querySelector("tbody")?.textContent).toContain(metadata.summary);
    expect(mocks.listMemories).not.toHaveBeenCalled();
    await click("Next page");
    expect(mocks.listMemories).toHaveBeenLastCalledWith({ data: { agentId: "agent", filters: { cursor: initialMemories.nextCursor } } });
    await click("Apply filters");
    expect(mocks.listMemories).toHaveBeenLastCalledWith({ data: { agentId: "agent", filters: {} } });
  });
  it("shows fresh status in green beside the detail title copy button", async () => {
    detail = { ...detail, status: "fresh" };
    await renderDetail();
    const copy = container.querySelector('button[aria-label="Copy memory reference"]')!;
    expect(copy.nextElementSibling?.textContent).toBe("fresh");
    expect(copy.nextElementSibling?.className).toContain("text-emerald-300");
  });
  it("renders memories in a table with a contained heading and collapsed filters", async () => {
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} />));
    const heading = container.querySelector("h1")!;
    expect(heading.textContent).toBe("Memories");
    expect(heading.className).toContain("text-lg");
    expect(heading.closest("section")?.className).toContain("p-6");
    const filters = container.querySelector("details")!;
    expect(filters.open).toBe(false);
    expect(filters.querySelector("summary")?.textContent).toBe("Filter");
    const table = container.querySelector('table[aria-label="Memories"]')!;
    expect([...table.querySelectorAll("th")].map((cell) => cell.textContent)).toEqual(["Memory", "Kind", "Instrument", "Topic / interval", "Status", "Producer", "Created"]);
    expect(table.querySelectorAll("tbody tr")).toHaveLength(1);
    expect(table.textContent).toContain(metadata.summary);
    for (const column of [1, 3]) {
      expect(table.querySelectorAll("tbody td")[column]!.querySelector("span")?.className).toContain("rounded-full");
    }
    expect(table.querySelector("a")?.getAttribute("href")).toBe("/agents/agent/memories/m1");
    expect(container.querySelector('[aria-label="Memory detail"]')).toBeNull();
    await click("Next page");
    expect(mocks.listMemories).toHaveBeenLastCalledWith({ data: { agentId: "agent", filters: { cursor: { createdAt: 1000, ref: "m1" } } } });
  });
  it("colors memory statuses and displays instrument icons with an agent-wide fallback", async () => {
    const statuses = ["fresh", "stale", "retracted", "invalid_evidence", "durable"];
    mocks.listMemories.mockResolvedValue({ ok: true, data: { items: statuses.map((status, index) => ({ ...metadata, ref: `m${index + 1}`, status, instrument: index === 0 ? "BTC" : null })), nextCursor: null } });
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} />));
    const rows = [...container.querySelectorAll("tbody tr")];
    const colors = ["text-emerald-300", "text-yellow-300", "text-zinc-400", "text-red-300", "text-violet-300"];
    rows.forEach((row, index) => {
      const badge = row.querySelectorAll("td")[4]!.querySelector("span")!;
      expect(badge.textContent).toBe(statuses[index]);
      expect(badge.className).toContain(colors[index]);
    });
    const instrument = rows[0]!.querySelectorAll("td")[2]!;
    expect(instrument.querySelector("img")?.getAttribute("src")).toBe("/currency/BTC.svg");
    expect(instrument.querySelector(".sr-only")?.textContent).toBe("BTC");
    expect(rows[1]!.querySelectorAll("td")[2]!.textContent).toBe("Agent-wide");
  });
  it("shows creation time ago and toggles to UTC and back on click", async () => {
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} />));
    const toggle = container.querySelector<HTMLButtonElement>('button[aria-label="Show UTC creation timestamp"]')!;
    expect(toggle.textContent).toContain("ago");
    expect(toggle.querySelector("time")?.dateTime).toBe("1970-01-01T00:00:01.000Z");
    await act(async () => toggle.click());
    expect(toggle.textContent).toBe("1970-01-01 00:00:01 UTC");
    expect(toggle.getAttribute("aria-label")).toBe("Show relative creation time");
    await act(async () => toggle.click());
    expect(toggle.textContent).toContain("ago");
    expect(toggle.getAttribute("aria-label")).toBe("Show UTC creation timestamp");
  });
  it("excludes archived sub-agents from producer and insight scope options while preserving publication names", async () => {
    const worker: SubagentRow = { id: "active", agentId: "agent", kind: "worker", name: "Active analyst", description: "", instructions: "", provider: "test", modelId: "test", thinkingLevel: "", tools: [], templateOrigin: null, enabled: true, archived: false, delegatable: true, persistent: false, conversationId: null, schedule: "", delaySeconds: 1, nextRunAt: null, completion: "save", timeoutSeconds: 900, createdAt: 0, updatedAt: 0 };
    const archived = { ...worker, id: "archived", name: "Archived analyst", archived: true };
    mocks.listMemories.mockResolvedValue({ ok: true, data: { items: [{ ...metadata, producerSubagentId: archived.id }], nextCursor: null } });
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[worker, archived]} />));
    const producer = [...container.querySelectorAll("select")].find((select) => select.parentElement?.textContent?.startsWith("Producer"))!;
    expect([...producer.options].map((option) => [option.value, option.textContent])).toEqual([["", "All"], ["active", "Active analyst"]]);
    expect(container.querySelector("tbody")?.textContent).toContain("Archived analyst");
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[worker, archived, { ...worker, id: "root", kind: "orchestrator", name: "Root" }]} initialTab="insights" />));
    const scope = container.querySelector('nav[aria-label="Insights scope"]')!;
    expect([...scope.querySelectorAll("button")].map((button) => button.textContent)).toEqual(["Global", "Active analyst"]);
    expect(scope.querySelector('[aria-current="true"]')?.textContent).toBe("Global");
    mocks.getInsights.mockResolvedValueOnce({ ok: true, data: { ...data, current: { ...current, content: "Analyst guidance" } } });
    await click("Active analyst");
    expect(mocks.getInsights).toHaveBeenLastCalledWith({ data: { agentId: "agent", scope: { kind: "worker", subagentId: "active" }, history: true } });
    expect(scope.querySelector('[aria-current="true"]')?.textContent).toBe("Active analyst");
    expect(container.querySelector('[aria-label="Latest insights"]')?.textContent).toContain("Analyst guidance");
    await click("Global");
    expect(mocks.getInsights).toHaveBeenLastCalledWith({ data: { agentId: "agent", scope: { kind: "agent" }, history: true } });
  });
  it("applies filters to a fresh page and displays an empty result", async () => {
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} />));
    await act(async () => { container.querySelector("summary")!.click(); });
    expect(container.querySelector("details")?.open).toBe(true);
    const kind = container.querySelector<HTMLSelectElement>('[aria-label="Memory kind"]')!;
    await act(async () => { kind.value = "research"; kind.dispatchEvent(new Event("change", { bubbles: true })); });
    await edit(container.querySelector<HTMLInputElement>('[aria-label="Search memory summaries"]')!, "support");
    expect(mocks.listMemories).toHaveBeenCalledTimes(1);
    mocks.listMemories.mockResolvedValue({ ok: true, data: { items: [], nextCursor: null } });
    await click("Apply filters");
    expect(mocks.listMemories).toHaveBeenLastCalledWith({ data: { agentId: "agent", filters: { kind: "research", query: "support" } } });
    expect(container.querySelector("tbody")?.textContent).toContain("No memories found");
    expect([...container.querySelectorAll("button")].find((button) => button.textContent === "Next page")?.disabled).toBe(true);
    await act(async () => { kind.value = ""; kind.dispatchEvent(new Event("change", { bubbles: true })); });
    await edit(container.querySelector<HTMLInputElement>('[aria-label="Search memory summaries"]')!, "");
    await click("Apply filters");
    expect(mocks.listMemories).toHaveBeenLastCalledWith({ data: { agentId: "agent", filters: {} } });
  });
  it("previews supporting artifacts in a modal with safe content, pagination, attachments and explicit downloads", async () => {
    detail = { ...detail, content: "Report", payload: {}, links: [], artifactIds: ["report"] };
    const preview = { id: "report", title: "Research report", description: "Evidence", mediaType: "text/plain", content: "<script>untrusted</script>", nextOffset: 100, attachments: [{ id: "chart", title: "Chart", mediaType: "image/png" }] };
    mocks.readArtifact.mockResolvedValue({ ok: true, data: preview });
    await renderDetail();
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
    detail = { ...detail, content: "Report", payload: {}, links: [], artifactIds: ["report"] };
    const preview = { id: "report", title: "Report", description: "", mediaType: "text/plain", content: "Evidence", nextOffset: null, attachments: [], expiresAt: 1000, protectedByMemory: false };
    mocks.readArtifact.mockResolvedValue({ ok: true, data: preview });
    mocks.changeArtifact.mockResolvedValue({ ok: true, data: { updated: true } });
    await renderDetail();
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
    detail = { ...detail, content: "Report", payload: {}, links: [], artifactIds: ["missing"] };
    mocks.readArtifact.mockResolvedValue({ ok: false, error: "Artifact not found" });
    await renderDetail();
    await click("Supporting artifact missing");
    expect(document.querySelector("dialog [role=alert]")?.textContent).toBe("Artifact not found");
    await act(async () => { document.querySelector<HTMLButtonElement>("dialog button")!.click(); });
    expect(document.querySelector("dialog")).toBeNull();
  });
  it("renders prose-only research without empty data and offers only host-derived status filters", async () => {
    detail = { ...detail, content: "Mixed evidence near support; evaluate before acting.", payload: {}, links: [], artifactIds: [] };
    await renderDetail();
    expect(container.textContent).toContain("Mixed evidence near support; evaluate before acting.");
    expect(container.querySelector('[aria-label="Additional memory data"]')).toBeNull();
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} />));
    const status = [...container.querySelectorAll("select")].find((select) => select.parentElement?.textContent?.startsWith("Status"))!;
    expect([...status.options].map((option) => option.value)).toEqual(["", "fresh", "stale", "retracted", "invalid_evidence", "durable"]);
  });
  it("displays arbitrary research data safely without enforcing signal labels", async () => {
    detail = { ...detail, content: "Report", payload: { bias: "bearish-neutral", custom: { note: "<script>untrusted</script>" } }, links: [], artifactIds: [] };
    await renderDetail();
    const payload = container.querySelector('[aria-label="Additional memory data"]');
    expect(payload?.textContent).toContain("bearish-neutral");
    expect(payload?.textContent).toContain("<script>untrusted</script>");
    expect(container.querySelector("script")).toBeNull();
  });
  it("shows only the latest insights read-only with collapsed history at the bottom", async () => {
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} initialTab="insights" version={1} />));
    expect(container.querySelector("h1")?.textContent).toBe("Insights");
    expect(container.textContent).toContain("Insights are read-only, managed by a Review sub-agent");
    expect(container.textContent).toContain("Latest · v1");
    expect(container.querySelector("pre")?.textContent).toBe("Current guidance");
    expect(container.querySelector("input, textarea, select")).toBeNull();
    expect(mocks.getInsights).toHaveBeenLastCalledWith({ data: { agentId: "agent", scope: { kind: "agent" }, history: true } });
    const history = container.querySelector("details")!;
    expect(history.open).toBe(false);
    expect(history.querySelector("summary")?.textContent).toBe("Revision history");
    expect(history.textContent).toContain("Initial review");
    expect(container.querySelector("pre")!.compareDocumentPosition(history) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
    expect(mocks.replaceInsights).not.toHaveBeenCalled();
  });
  it("paginates revision metadata without replacing the latest content", async () => {
    mocks.getInsights.mockResolvedValueOnce({ ok: true, data: { ...data, history: { ...data.history, nextBeforeVersion: 1 } } });
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} initialTab="insights" />));
    await click("Older revisions");
    expect(mocks.getInsights).toHaveBeenLastCalledWith({ data: { agentId: "agent", scope: { kind: "agent" }, history: true, beforeVersion: 1 } });
    expect(container.querySelector("pre")?.textContent).toBe("Current guidance");
  });
  it("shows a simple empty state and insight loading errors", async () => {
    mocks.getInsights.mockResolvedValueOnce({ ok: true, data: { current: { ...current, version: 0, content: "" }, head: null, revision: null, history: { items: [], nextBeforeVersion: null } } });
    await act(async () => root.render(<AgentMemoriesPage agentId="agent" workers={[]} initialTab="insights" />));
    expect(container.querySelector("pre")?.textContent).toBe("No insights yet.");
    expect(container.querySelector("details")?.textContent).toContain("No revisions yet.");
    mocks.getInsights.mockResolvedValueOnce({ ok: false, error: "Unable to read insights" });
    await act(async () => root.render(<AgentMemoriesPage key="error" agentId="agent" workers={[]} initialTab="insights" />));
    expect(container.querySelector('[role="alert"]')?.textContent).toBe("Unable to read insights");
    expect(container.querySelector('[aria-label="Latest insights"]')?.textContent).toContain("Insights unavailable.");
  });
});
