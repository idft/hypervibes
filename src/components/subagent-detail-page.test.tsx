// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { type QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createTestQueryClient } from "../test/query.js";
import type { SubagentDetail } from "../web/subagent-queries.js";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SubagentRow } from "../db/subagent-schema.js";
import { hyperliquidToolNames, mainToolNames, taToolNames } from "../agents/subagent-input.js";
const mocks = vi.hoisted(() => ({ invalidate: vi.fn(), navigate: vi.fn(), success: vi.fn(), saveSubagent: vi.fn(), saveSubagentSection: vi.fn(), saveSubagentAssignments: vi.fn(), saveSubagentIdentity: vi.fn(), saveSubagentTools: vi.fn(), getSubagent: vi.fn(), runSubagent: vi.fn(), archiveSubagent: vi.fn(), unarchiveSubagent: vi.fn(), stopSubagentRun: vi.fn(), listSubagents: vi.fn() }));
vi.mock("../web/subagents.js", () => mocks);
vi.mock("sonner", () => ({ toast: { success: mocks.success } }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => ({ invalidate: mocks.invalidate, navigate: mocks.navigate }), Link: ({ to, params, children, ...props }: { to: string; params: Record<string, string>; children: React.ReactNode }) => <a {...props} href={to.replace(/\$(\w+)/g, (_match, key: string) => encodeURIComponent(params[key]!))}>{children}</a> }));
import { SubagentDetailPage as SubagentDetailPageComponent } from "./subagent-detail-page.js";
import { SubagentsPage as SubagentsPageComponent } from "./subagents-page.js";
let queryClient: QueryClient;
function detailPage({ initial, ...props }: Omit<ComponentProps<typeof SubagentDetailPageComponent>, "subagentId"> & { key?: string; initial: Pick<SubagentDetail, "definition" | "runs" | "schedulerError"> & Omit<Partial<SubagentDetail>, "workers"> & { workers?: { id: string; name: string; archived?: boolean; tools?: string[] }[] } }) {
  queryClient.setQueryData(["subagents", props.agentId, "detail", initial.definition.id], initial);
  return <QueryClientProvider client={queryClient}><SubagentDetailPageComponent {...props} subagentId={initial.definition.id} /></QueryClientProvider>;
}
function subagentsPage({ agentId, definitions }: { agentId: string; definitions: SubagentRow[] }) {
  queryClient.setQueryData(["subagents", agentId, "list"], definitions);
  return <QueryClientProvider client={queryClient}><SubagentsPageComponent agentId={agentId} /></QueryClientProvider>;
}
import { ScheduleConfig } from "./subagent-config.js";
const definition: SubagentRow = { id: "worker:fixture", agentId: "agent-fixture", kind: "worker", name: "Research", description: "Find evidence", instructions: "Research carefully", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], triggers: { inboxEnabled: false, workerDelegates: [], researchSources: [], fills: false, orderChanges: false }, templateOrigin: null, enabled: true, archived: false, delegatable: true, persistent: false, conversationId: null, schedule: "", delaySeconds: 30, nextRunAt: null, completion: "wake", timeoutSeconds: 900, createdAt: 1, updatedAt: 1 };
class HistorySource extends EventTarget {
  static instances: HistorySource[] = [];
  onerror: (() => void) | null = null;
  close = vi.fn();
  constructor(readonly url: string) { super(); HistorySource.instances.push(this); }
}
const choice = (provider: string, modelId: string, name: string, thinkingLevels: string[]) => ({ provider, modelId, name, providerName: provider, thinkingLevels, contextWindow: 200_000, inputCost: 3, outputCost: 15, reasoning: true });
const choices = [choice("faux", "faux-1", "Faux", ["high"])];
describe("subagent pages", () => {
  let root: Root, container: HTMLDivElement;
  const initial = (row = definition) => ({ definition: row, runs: [], schedulerError: null });
  const render = async (row = definition) => act(async () => root.render(detailPage({ key: row.kind, agentId: row.agentId, initial: initial(row), choices })));
  const click = async (text: string) => act(async () => { [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === text || button.getAttribute("aria-label") === text)!.click(); });
  const editInstructions = async (text: string) => act(async () => {
    const input = container.querySelector("textarea")!;
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  beforeEach(() => { vi.resetAllMocks(); queryClient = createTestQueryClient({ staleTime: Infinity }); mocks.listSubagents.mockImplementation(({ data }: { data: { agentId: string } }) => Promise.resolve(queryClient.getQueryData(["subagents", data.agentId, "list"]))); HistorySource.instances = []; vi.stubGlobal("EventSource", HistorySource); container = document.createElement("div"); document.body.append(container); root = createRoot(container); mocks.getSubagent.mockResolvedValue(initial()); mocks.saveSubagentSection.mockImplementation(({ data }: { data: { settings: Partial<SubagentRow> } }) => Promise.resolve({ ...definition, ...data.settings })); mocks.saveSubagentAssignments.mockImplementation(({ data }: { data: { settings: Partial<SubagentRow> } }) => Promise.resolve({ ...definition, ...data.settings })); mocks.saveSubagentIdentity.mockImplementation(({ data }: { data: { name: string; description: string } }) => Promise.resolve({ ...definition, name: data.name, description: data.description })); mocks.saveSubagentTools.mockImplementation(({ data }: { data: { tools: string[] } }) => Promise.resolve({ ...definition, tools: data.tools })); });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
  it("paginates live history without refresh controls, ignores old streams and cleans up on navigation", async () => {
    await render();
    expect(container.textContent).not.toContain("latest 50");
    expect([...container.querySelectorAll("button")].some((button) => button.textContent === "Refresh")).toBe(false);
    expect(container.querySelector('[aria-label="Run history pagination"]')).toBeNull();
    const first = HistorySource.instances.at(-1)!;
    expect(first.url).toBe("/api/agents/agent-fixture/sub-agents/worker%3Afixture/runs/events?page=1");
    const summary = { id: "run-1", subagentId: definition.id, status: "running", source: "manual", task: "Live assignment", createdAt: Date.now() };
    const snapshot = (page: number, total = 51) => ({ runs: [{ ...summary, task: `Page ${page} assignment` }], pagination: { page, pages: Math.ceil(total / 50), total, pageSize: 50 }, schedulerError: null });
    const push = (source: HistorySource, page: number, total = 51) => act(async () => { source.dispatchEvent(new MessageEvent("runs", { data: JSON.stringify(snapshot(page, total)) })); });
    await push(first, 1);
    expect(container.textContent).toContain("Page 1 of 2");
    expect(container.textContent).toContain("Page 1 assignment");
    const previous = [...container.querySelectorAll("button")].find((button) => button.textContent === "Previous")!;
    expect(previous.disabled).toBe(true);
    await click("Next");
    const second = HistorySource.instances.at(-1)!;
    expect(second.url).toContain("page=2"); expect(first.close).toHaveBeenCalledOnce();
    await push(first, 1, 999); // Late updates from the closed page cannot overwrite it.
    expect(container.textContent).not.toContain("999 runs");
    await push(second, 2);
    expect(container.textContent).toContain("Page 2 of 2");
    expect(container.textContent).toContain("Page 2 assignment");
    expect([...container.querySelectorAll("button")].find((button) => button.textContent === "Next")!.disabled).toBe(true);
    await push(second, 2, 52);
    expect(container.textContent).toContain("52 runs");
    expect(HistorySource.instances).toHaveLength(2); // New runs do not jump to page one.
    await act(async () => second.onerror?.());
    expect(container.textContent).toContain("Live updates interrupted");
    await push(second, 2, 53);
    expect(container.textContent).not.toContain("Live updates interrupted");
    mocks.stopSubagentRun.mockResolvedValue(undefined);
    await click("Stop assignment");
    expect(mocks.stopSubagentRun).toHaveBeenCalledWith({ data: { agentId: definition.agentId, runId: summary.id } });
    expect(mocks.getSubagent).not.toHaveBeenCalled();
    await render({ ...definition, kind: "orchestrator" });
    expect(second.close).toHaveBeenCalledOnce();
  });

  it("lists the protected orchestrator and links workers to detail pages", () => {
    const html = renderToStaticMarkup(subagentsPage({ agentId: definition.agentId, definitions: [{ ...definition, id: "main:fixture", kind: "orchestrator", name: "Orchestrator" }, definition] }));
    expect(html).toContain("Main conversation"); expect(html).not.toContain("Protected"); expect(html).toContain("Create subagent");
    expect(html).toContain("/agents/agent-fixture/sub-agents/worker%3Afixture"); expect(html).toContain("Fresh context per assignment");
  });
  it("hides archived workers by default and lets users show or hide them", async () => {
    const archived = { ...definition, id: "worker:archived", name: "Old research", archived: true, enabled: false };
    await act(async () => root.render(subagentsPage({ agentId: definition.agentId, definitions: [definition, archived] })));
    expect(container.textContent).not.toContain("Configure your orchestrator");
    expect(container.textContent).not.toContain("Old research");
    expect(container.querySelector("button")?.getAttribute("aria-pressed")).toBe("false");
    await click("Show archived (1)");
    expect(container.textContent).toContain("Old research");
    const badge = [...container.querySelectorAll("span")].find((span) => span.textContent === "Archived")!;
    expect(badge.className).toContain("text-amber-200");
    expect(badge.className).toContain("font-semibold");
    expect(badge.className).toContain("border-amber-500/60");
    expect(container.querySelector("button")?.getAttribute("aria-pressed")).toBe("true");
    await click("Hide archived");
    expect(container.textContent).not.toContain("Old research");
    expect(container.textContent).toContain("Research");
  });
  it("toggles workers on the index without nesting controls in links", async () => {
    await act(async () => root.render(subagentsPage({ agentId: definition.agentId, definitions: [definition] })));
    const toggle = container.querySelector<HTMLButtonElement>('[aria-label="Pause sub-agent"]')!;
    expect(toggle.closest("a")).toBeNull();
    await click("Pause sub-agent");
    expect(mocks.saveSubagentSection).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, section: "enabled", settings: { enabled: false } } });
    expect(container.querySelector('[aria-label="Enable sub-agent"]')?.textContent).toBe("Paused");
    expect(mocks.success).toHaveBeenCalledWith("Sub-agent paused");
    await click("Enable sub-agent");
    expect(container.querySelector('[aria-label="Pause sub-agent"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(mocks.invalidate).not.toHaveBeenCalled();
    expect(mocks.navigate).not.toHaveBeenCalled();
  });
  it("keeps index toggle state on failure and excludes protected or archived agents", async () => {
    mocks.saveSubagentSection.mockRejectedValue(new Error("Unable to save"));
    await act(async () => root.render(subagentsPage({ agentId: definition.agentId, definitions: [definition, { ...definition, id: "main", kind: "orchestrator" }, { ...definition, id: "old", archived: true }] })));
    await click("Show archived (1)");
    expect(container.querySelectorAll('[aria-label="Pause sub-agent"]')).toHaveLength(1);
    await click("Pause sub-agent");
    expect(container.querySelector('[role="alert"]')?.textContent).toBe("Unable to save");
    expect(container.querySelector('[aria-label="Pause sub-agent"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(mocks.invalidate).not.toHaveBeenCalled();
  });
  it("unarchives a worker outside the disabled settings and leaves it paused", async () => {
    const restored = { ...definition, enabled: false, nextRunAt: null, updatedAt: 2 };
    mocks.unarchiveSubagent.mockResolvedValue(restored);
    await render({ ...definition, archived: true, enabled: false });
    const restore = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Unarchive")!;
    expect(restore.closest("fieldset")).toBeNull();
    expect(restore.disabled).toBe(false);
    await click("Unarchive");
    expect(mocks.unarchiveSubagent).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id } });
    expect(container.querySelector("fieldset")?.disabled).toBe(false);
    expect(container.querySelector('[aria-label="Enable sub-agent"]')).not.toBeNull();
    expect(container.textContent).not.toContain("Unarchive");
    expect(mocks.success).toHaveBeenCalledWith("Sub-agent unarchived", { description: "Restored paused. Enable it when ready." });
    expect(mocks.invalidate).not.toHaveBeenCalled();
    expect(mocks.navigate).not.toHaveBeenCalled();
  });
  it("keeps archived settings disabled if unarchiving fails", async () => {
    mocks.unarchiveSubagent.mockRejectedValue(new Error("Unable to restore"));
    await render({ ...definition, archived: true, enabled: false });
    await click("Unarchive");
    expect(container.querySelector('[role="alert"]')?.textContent).toBe("Unable to restore");
    expect(container.querySelector("fieldset")?.disabled).toBe(true);
    expect(container.textContent).toContain("Unarchive");
    expect(mocks.invalidate).not.toHaveBeenCalled();
  });
  it("redirects to the refreshed index after archiving", async () => {
    await render(); const browserConfirm = vi.spyOn(window, "confirm");
    mocks.archiveSubagent.mockResolvedValue(undefined);
    await click("Archive worker");
    const dialog = container.querySelector("dialog")!;
    expect(dialog.open).toBe(true); expect(dialog.textContent).toContain("Research");
    expect(dialog.getAttribute("aria-labelledby")).toBeTruthy();
    expect(mocks.archiveSubagent).not.toHaveBeenCalled(); expect(browserConfirm).not.toHaveBeenCalled();
    await click("Archive sub-agent");
    expect(mocks.archiveSubagent).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id } });
    expect(mocks.invalidate).not.toHaveBeenCalled();
    expect(mocks.navigate).toHaveBeenCalledWith({ to: "/agents/$agentId/sub-agents", params: { agentId: definition.agentId } });
    expect(mocks.getSubagent).not.toHaveBeenCalled();
    expect(mocks.success).toHaveBeenCalledWith("Sub-agent archived", { description: "History is preserved." });
  });
  it("stays on the detail page when archiving fails", async () => {
    await render();
    mocks.archiveSubagent.mockRejectedValue(new Error("Stop the active assignment first"));
    await click("Archive worker"); await click("Archive sub-agent");
    expect(container.querySelector("[role=alert]")?.textContent).toContain("Stop the active assignment");
    expect(mocks.navigate).not.toHaveBeenCalled(); expect(mocks.invalidate).not.toHaveBeenCalled();
  });
  it("dismisses the archive modal with Cancel or Escape without archiving", async () => {
    await render();
    await click("Archive worker"); await click("Cancel");
    expect(container.querySelector("dialog")).toBeNull();
    await click("Archive worker");
    await act(async () => { container.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })); });
    expect(container.querySelector("dialog")).toBeNull();
    expect(mocks.archiveSubagent).not.toHaveBeenCalled(); expect(mocks.navigate).not.toHaveBeenCalled();
  });
  it("configures event triggers separately from scheduling and preserves selected sources", async () => {
    await act(async () => root.render(detailPage({ agentId: definition.agentId, initial: { ...initial(), workers: [{ id: definition.id, name: definition.name }, { id: "worker:analyst", name: "Analyst" }] }, choices })));
    await click("Edit triggers");
    const toggle = async (label: string) => act(async () => { const input = [...document.querySelectorAll("dialog label")].find((entry) => entry.textContent === label)!.querySelector("input")!; input.click(); });
    await toggle("Enable event triggers");
    const analystInputs = [...document.querySelectorAll<HTMLLabelElement>("dialog label")].filter((label) => label.textContent === "Analyst").map((label) => label.querySelector("input")!);
    expect(analystInputs).toHaveLength(2);
    await act(async () => { analystInputs[0]!.click(); analystInputs[1]!.click(); });
    await toggle("Order fills — partial or complete");
    await toggle("Order status changes — canceled, rejected, expired");
    await click("Apply");
    expect(mocks.saveSubagentSection).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, section: "triggers", settings: { triggers: { inboxEnabled: true, workerDelegates: ["worker:analyst"], researchSources: ["worker:analyst"], fills: true, orderChanges: true } } } });
    expect(container.textContent).toContain("Event-driven inbox enabled");
    expect(container.textContent).toContain("No schedule");
  });
  it.each([
    { tools: [], warns: true },
    { tools: ["list_subagents"], warns: true },
    { tools: ["delegate_subagent"], warns: false },
    { tools: ["request_trader_evaluation"], warns: false },
  ])("warns only for approved callers missing delegation tools: $tools", async ({ tools, warns }) => {
    const triggers = { ...definition.triggers, inboxEnabled: true, researchSources: ["worker:analyst"] };
    await act(async () => root.render(detailPage({ agentId: definition.agentId, initial: { ...initial(), definition: { ...definition, triggers }, workers: [{ id: "worker:analyst", name: "Analyst", tools }] }, choices })));
    await click("Edit triggers");
    const dialog = document.querySelector("dialog")!;
    expect(dialog.textContent).not.toContain("Multiple workers may subscribe");
    expect(dialog.textContent).not.toContain("Tasks can stand alone");
    expect(dialog.querySelector('[role="status"]')).toBeNull(); // Research subscriptions do not require delegation.
    const caller = [...dialog.querySelectorAll("fieldset")].find((fieldset) => fieldset.querySelector("legend")?.textContent === "Allow research delegation from")!.querySelector("input")!;
    await act(async () => caller.click());
    expect(dialog.querySelector('[role="status"]')?.textContent ?? null).toBe(warns ? "Analyst cannot delegate: enable delegate_subagent in its Tools." : null);
    expect([...dialog.querySelectorAll("button")].find((button) => button.textContent === "Apply")!.disabled).toBe(false);
    await act(async () => caller.click());
    expect(dialog.querySelector('[role="status"]')).toBeNull();
  });
  it("shows archived selected trigger sources and lets the operator remove them", async () => {
    const triggers = { ...definition.triggers, inboxEnabled: true, researchSources: ["worker:archived"] };
    await act(async () => root.render(detailPage({ agentId: definition.agentId, initial: { ...initial(), definition: { ...definition, triggers }, workers: [{ id: "worker:archived", name: "Old analyst", archived: true }] }, choices })));
    await click("Edit triggers");
    const labels = [...document.querySelectorAll("dialog label")].filter((entry) => entry.textContent === "Old analyst (archived)");
    expect(labels).toHaveLength(2);
    const selected = labels[0]!.querySelector("input")!;
    expect(selected.checked).toBe(true);
    expect(selected.disabled).toBe(false);
    expect(labels[1]!.querySelector("input")!.disabled).toBe(true);
    await act(async () => selected.click());
    await click("Apply");
    expect(mocks.saveSubagentSection).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, section: "triggers", settings: { triggers: { ...triggers, researchSources: [] } } } });
  });
  it("shows queued event provenance and allows discarding a pending request", async () => {
    await render();
    await act(async () => HistorySource.instances.at(-1)!.dispatchEvent(new MessageEvent("runs", { data: JSON.stringify({ runs: [], pagination: { page: 1, pages: 1, total: 0, pageSize: 50 }, schedulerError: null, inbox: [{ id: "request-1", intent: "account_event", reason: "fill", preview: "Partial BTC fill", createdAt: 1, discardedReason: null }] }) })));
    expect(container.textContent).toContain("Partial BTC fill");
    expect(container.textContent).toContain("account_event");
    mocks.stopSubagentRun.mockResolvedValue(undefined);
    await click("Discard request");
    expect(mocks.stopSubagentRun).toHaveBeenCalledWith({ data: { agentId: definition.agentId, runId: "request-1" } });
  });
  it("edits the orchestrator model, thinking and tools without offering deletion or schedules", async () => {
    const main = { ...definition, id: "main:fixture", name: "Orchestrator", kind: "orchestrator" as const, tools: mainToolNames };
    await render(main); mocks.saveSubagentSection.mockResolvedValue({ ...main, thinkingLevel: "high" });
    expect(container.textContent).not.toContain("cannot be disabled, archived, or deleted"); expect(container.textContent).not.toContain("Archive worker"); expect(container.textContent).not.toContain("UTC candle close");
    expect(container.textContent).not.toContain("Manage providers");
    expect(container.querySelector("select")).toBeNull();
    expect(container.querySelector('[aria-label="Choose model"]')?.textContent).toContain("Thinking: Default");
    await click("Choose model");
    const thinking = document.querySelector("dialog select") as HTMLSelectElement;
    await act(async () => { thinking.value = "high"; thinking.dispatchEvent(new Event("change", { bubbles: true })); });
    await click("Apply");
    expect(container.querySelector('[aria-label="Choose model"]')?.textContent).toContain("Thinking: high");
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
    expect(mocks.saveSubagentSection).toHaveBeenCalledWith({ data: { agentId: main.agentId, id: main.id, section: "model", settings: { provider: "faux", modelId: "faux-1", thinkingLevel: "high" } } });
    expect(mocks.success).toHaveBeenCalledWith("Model updated");
    expect(container.textContent).not.toContain("Settings saved");
  });
  it("places tools beside stacked assignment and schedule settings above instructions", async () => {
    await render();
    const sections = [...container.querySelectorAll("section")];
    const tools = sections.find((section) => section.querySelector("h3")?.textContent === "Tools")!;
    const assignments = sections.find((section) => section.querySelector("h3")?.textContent === "Assignments & context")!;
    const schedule = sections.find((section) => section.querySelector("h3")?.textContent === "Schedule")!;
    const grid = tools.parentElement!.parentElement!;
    expect(grid.classList.contains("md:grid-cols-2")).toBe(true);
    expect(grid.children).toHaveLength(2);
    expect(grid.children[0]!.contains(tools)).toBe(true);
    expect(grid.children[1]!.children[0]).toBe(assignments);
    expect(grid.children[1]!.children[1]!.querySelector("h3")!.textContent).toBe("Event triggers");
    expect(grid.children[1]!.children[2]).toBe(schedule);
    const instructions = container.querySelector("textarea")!.closest("label")!;
    expect(grid.nextElementSibling).toBe(instructions);
  });
  it("offers candle-close scheduling, lifetime and completion choices without recursive worker tools", async () => {
    await render(); expect(container.textContent).toContain("Orchestrator delegation allowed"); expect(container.textContent).toContain("Fresh context per assignment"); expect(container.textContent).not.toContain("delegate_subagent");
    expect(container.textContent).toContain("No schedule");
    expect(container.querySelectorAll("select")).toHaveLength(0); // No workflow/purpose selects; tools and model choices only.
    await click("Edit schedule");
    const schedule = [...document.querySelectorAll<HTMLSelectElement>("dialog select")].find((select) => [...select.options].some((option) => option.value === "1h"))!;
    await act(async () => { schedule.value = "1h"; schedule.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(document.querySelector("dialog")!.textContent).toContain("Delay after candle close");
    expect(document.querySelector("dialog")!.textContent).toContain("Save only");
    expect(document.querySelector("dialog")!.textContent).toContain("without a model call");
    await click("Apply");
    expect(document.querySelector("dialog")).toBeNull();
    expect(container.textContent).toContain("Every 1h candle close (UTC) · 1s delay");
    expect(mocks.saveSubagentSection).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, section: "schedule", settings: { schedule: "1h", delaySeconds: 1, completion: "wake" } } });
  });
  it("preserves an existing schedule's configured delay", async () => {
    await render({ ...definition, schedule: "15m", delaySeconds: 45 });
    await click("Edit schedule");
    expect(document.querySelector<HTMLInputElement>('dialog input[type="number"]')!.value).toBe("45");
    await click("Apply");
    expect(mocks.saveSubagentSection).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, section: "schedule", settings: { schedule: "15m", delaySeconds: 45, completion: "wake" } } });
  });
  it("searches models by provider, name and ID and only applies confirmed choices", async () => {
    await act(async () => root.render(detailPage({ agentId: definition.agentId, initial: initial({ ...definition, thinkingLevel: "high" }), choices: [...choices, choice("other", "second-2", "Second", [])] })));
    await click("Choose model");
    const search = document.querySelector("dialog input[type=search]")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, "second-2");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(document.querySelector("dialog")!.textContent).toContain("Second");
    expect(document.querySelector("dialog")!.textContent).not.toContain("Faux");
    await act(async () => { (document.querySelector("dialog input[type=radio]") as HTMLInputElement).click(); });
    await click("Cancel");
    expect(container.textContent).toContain("faux / faux-1");
    await click("Choose model");
    expect(document.querySelector("dialog")!.textContent).toContain("200K ctx · reasoning · $3/$15");
    await act(async () => { (document.querySelectorAll("dialog input[type=radio]")[1] as HTMLInputElement).click(); });
    await click("Apply");
    expect(container.textContent).toContain("other / second-2");
    expect(container.textContent).not.toContain("Thinking level");
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
    expect(mocks.saveSubagentSection.mock.calls[0]![0].data).toMatchObject({ section: "model", settings: { provider: "other", modelId: "second-2", thinkingLevel: "" } });
  });
  it("preserves thinking on reopen and discards unconfirmed changes", async () => {
    await render({ ...definition, thinkingLevel: "high" });
    await click("Choose model");
    const thinking = document.querySelector("dialog select") as HTMLSelectElement;
    expect(thinking.value).toBe("high");
    await act(async () => { thinking.value = ""; thinking.dispatchEvent(new Event("change", { bubbles: true })); });
    await click("Cancel");
    await click("Choose model");
    expect((document.querySelector("dialog select") as HTMLSelectElement).value).toBe("high");
    await click("Apply");
    expect(mocks.saveSubagentSection.mock.calls[0]![0].data.settings.thinkingLevel).toBe("high");
  });
  it("groups and searches delegation tools for both workers and the orchestrator", async () => {
    await render();
    await click("Edit tools");
    expect(document.querySelector("dialog")!.textContent).toContain("Hyperliquid");
    expect(document.querySelector("dialog")!.textContent).toContain("delegate_subagent");
    expect(document.querySelector("dialog")!.textContent).toContain("list_subagents");
    expect(document.querySelector("dialog")!.textContent).toContain("get_subagent_run");
    expect(document.querySelector("dialog")!.textContent).toContain("list_subagent_runs");
    expect(document.querySelector("dialog")!.textContent).toContain("research_execution_audit");
    await act(async () => { (document.querySelector("dialog input[type=checkbox]") as HTMLInputElement).click(); });
    await click("Cancel");
    expect(container.textContent).toContain("No tools allowed");
    await click("Edit tools");
    await act(async () => { (document.querySelector("dialog input[type=checkbox]") as HTMLInputElement).click(); });
    await click("Apply");
    expect(container.textContent).toContain("1 tool allowed");
    expect(container.textContent).toContain("Hyperliquid: get_account");
    expect(mocks.saveSubagentTools).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, tools: ["get_account"] } });
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
    expect(mocks.success).toHaveBeenCalledWith("Tools updated");
    await render({ ...definition, kind: "orchestrator" });
    await click("Edit tools");
    expect(document.querySelector("dialog")!.textContent).toContain("HyperVibes · Subagents");
    const search = document.querySelector("dialog input[type=search]")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, "delegate");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(document.querySelectorAll("dialog input[type=checkbox]")).toHaveLength(1);
    expect(document.querySelector("dialog")!.textContent).toContain("delegate_subagent");
    await act(async () => { document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })); });
    expect(document.querySelector("dialog")).toBeNull();
  });
  it("groups instrument discovery under Hyperliquid without duplicating it in Research", async () => {
    await render({ ...definition, tools: ["list_instruments"] });
    expect(container.textContent).toContain("Hyperliquid: list_instruments");
    expect(container.textContent).not.toContain("Research · Code Mode & Artifacts:");
    await click("Edit tools");
    const groups = [...document.querySelectorAll("dialog fieldset")];
    const hyperliquid = groups.find((group) => group.querySelector(":scope > legend")?.textContent?.startsWith("Hyperliquid"))!;
    const research = groups.find((group) => group.querySelector(":scope > legend")?.textContent?.startsWith("Research"))!;
    expect(hyperliquid.textContent).toContain("list_instruments");
    expect(research.textContent).not.toContain("list_instruments");
    expect([...document.querySelectorAll("dialog label")].filter((label) => label.textContent === "list_instruments")).toHaveLength(1);
    await click("No tools in Hyperliquid");
    expect(document.querySelectorAll("dialog input[type=checkbox]:checked")).toHaveLength(0);
    await click("All tools in Research · Code Mode & Artifacts");
    const discovery = [...hyperliquid.querySelectorAll("label")].find((label) => label.textContent === "list_instruments")!;
    expect(discovery.querySelector("input")!.checked).toBe(false);
  });
  it("selects the separate TA group without enabling Code Mode, account or delegation tools", async () => {
    await render(); await click("Edit tools");
    expect(document.querySelector("dialog")!.textContent).toContain("Technical analysis · TA");
    expect(document.querySelector("dialog")!.textContent).not.toContain("research_ta");
    await click("All tools in Technical analysis · TA");
    expect(document.querySelectorAll("dialog input[type=checkbox]:checked")).toHaveLength(7);
    await click("Apply");
    expect(mocks.saveSubagentTools.mock.calls[0]![0].data.tools).toEqual([...taToolNames]);
    expect(container.textContent).toContain("Technical analysis · TA:");
    expect(container.textContent).not.toContain("Hyperliquid: get_account");
  });
  it("selects and clears all tools per extension even while searching", async () => {
    await render({ ...definition, kind: "orchestrator" });
    await click("Edit tools");
    await click("All tools in Hyperliquid");
    await click("All tools in Hyperliquid");
    await click("All tools in HyperVibes · Subagents");
    expect(document.querySelectorAll("dialog input[type=checkbox]:checked")).toHaveLength(hyperliquidToolNames.length + 4);
    const search = document.querySelector("dialog input[type=search]")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, "get_account");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(document.querySelectorAll("dialog input[type=checkbox]")).toHaveLength(1);
    await click("No tools in Hyperliquid");
    expect(document.querySelectorAll("dialog input[type=checkbox]:checked")).toHaveLength(0);
    expect(document.querySelector("dialog")!.textContent).toContain("4 selected");
    await click("All tools in Hyperliquid");
    expect(document.querySelector("dialog")!.textContent).toContain(`${hyperliquidToolNames.length + 4} selected`);
    await click("No tools in Hyperliquid");
    await click("Apply");
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
    expect(mocks.saveSubagentTools.mock.calls[0]![0].data.tools).toEqual(["list_subagents", "delegate_subagent", "list_subagent_runs", "get_subagent_run"]);
  });
  it("keeps failed tool edits open for retry without changing the saved summary", async () => {
    await render();
    mocks.saveSubagentTools.mockRejectedValueOnce(new Error("Stop the active assignment first"));
    await click("Edit tools"); await click("All tools in Hyperliquid"); await click("Apply");
    expect(document.querySelector("dialog [role=alert]")?.textContent).toContain("Stop the active assignment");
    expect(container.textContent).toContain("No tools allowed");
    expect(document.querySelectorAll("dialog input:checked")).toHaveLength(hyperliquidToolNames.length);
    await click("Apply");
    expect(document.querySelector("dialog")).toBeNull();
    expect(container.textContent).toContain(`${hyperliquidToolNames.length} tools allowed`);
  });
  it("saves tools without saving or discarding other unfinished form edits", async () => {
    await render();
    const instructions = container.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(instructions, "Unsaved instructions");
      instructions.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await click("Edit tools"); await click("All tools in Hyperliquid"); await click("Apply");
    expect(instructions.value).toBe("Unsaved instructions");
    expect(mocks.saveSubagentTools.mock.calls[0]![0].data).toEqual({ agentId: definition.agentId, id: definition.id, tools: [...hyperliquidToolNames] });
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
    await click("Save instructions");
    expect(mocks.saveSubagentSection.mock.calls[0]![0].data).toMatchObject({ section: "instructions", settings: { instructions: "Unsaved instructions" } });
    expect(container.textContent).toContain(`${hyperliquidToolNames.length} tools allowed`);
  });
  it("saves name and description immediately without saving unrelated edits", async () => {
    await render();
    const instructions = container.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(instructions, "Unfinished instructions");
      instructions.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(container.querySelector("h2")?.textContent).toBe("Research");
    expect(container.textContent).toContain("Find evidence");
    expect(container.querySelector('input[type="text"], input:not([type])')).toBeNull();
    await click("Edit name and description");
    expect(document.querySelector("dialog")?.textContent).not.toContain("Apply changes to the form");
    expect(document.querySelector("dialog")?.textContent).toContain("Human-readable description.");
    const inputs = document.querySelectorAll<HTMLInputElement>("dialog input");
    expect(inputs[0]!.value).toBe("Research");
    expect(inputs[0]!.required).toBe(true);
    expect(inputs[0]!.maxLength).toBe(80);
    expect(inputs[1]!.maxLength).toBe(500);
    await act(async () => {
      for (const [index, value] of ["New name", "New description"].entries()) {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(inputs[index], value);
        inputs[index]!.dispatchEvent(new Event("input", { bubbles: true }));
      }
    });
    await click("Apply");
    expect(document.querySelector("dialog")).toBeNull();
    expect(container.querySelector("h2")?.textContent).toBe("New name");
    expect(container.textContent).toContain("New description");
    expect(instructions.value).toBe("Unfinished instructions");
    expect(mocks.saveSubagentIdentity).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, name: "New name", description: "New description" } });
    expect(mocks.success).toHaveBeenCalledWith("Name and description updated");
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
    await click("Save instructions");
    expect(mocks.saveSubagentSection.mock.calls[0]![0].data.settings).toEqual({ instructions: "Unfinished instructions" });
    expect(container.querySelector("h2")?.textContent).toBe("New name");
  });
  it("discards identity edits on Cancel or Escape and rejects a blank name", async () => {
    await render();
    for (const dismiss of ["cancel", "escape"]) {
      await click("Edit name and description");
      const name = document.querySelector("dialog input")!;
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(name, "   ");
        name.dispatchEvent(new Event("input", { bubbles: true }));
      });
      expect([...document.querySelectorAll<HTMLButtonElement>("dialog button")].find((button) => button.textContent === "Apply")!.disabled).toBe(true);
      if (dismiss === "cancel") await click("Cancel");
      else await act(async () => { document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })); });
      expect(container.querySelector("h2")?.textContent).toBe("Research");
    }
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
  });
  it("retains failed identity edits for retry", async () => {
    await render();
    mocks.saveSubagentIdentity.mockRejectedValueOnce(new Error("Stop the active assignment first"));
    await click("Edit name and description");
    const name = document.querySelector("dialog input") as HTMLInputElement;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(name, "Updated name");
      name.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await click("Apply");
    expect(document.querySelector("dialog [role=alert]")?.textContent).toContain("Stop the active assignment");
    expect(name.value).toBe("Updated name");
    expect(container.querySelector("h2")?.textContent).toBe("Research");
    await click("Apply");
    expect(document.querySelector("dialog")).toBeNull();
    expect(container.querySelector("h2")?.textContent).toBe("Updated name");
  });
  it("disables identity editing for archived subagents", async () => {
    await render({ ...definition, archived: true });
    expect(container.querySelector('[aria-label="Edit name and description"]')?.matches(":disabled")).toBe(true);
  });
  it("automatically saves assignment options without saving unrelated edits", async () => {
    await render();
    expect(container.querySelector('input[type="checkbox"]')).toBeNull();
    const instructions = container.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(instructions, "Unfinished instructions");
      instructions.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await click("Edit assignments");
    await act(async () => {
      document.querySelectorAll<HTMLInputElement>('dialog input[type="checkbox"]').forEach((input) => input.click());
      const timeout = document.querySelector('dialog input[type="number"]')!;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(timeout, "600");
      timeout.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await click("Apply");
    expect(document.querySelector("dialog")).toBeNull();
    expect(container.textContent).toContain("Orchestrator delegation disabled");
    expect(container.querySelector('[aria-label="Pause sub-agent"]')?.textContent).toBe("Enabled");
    expect(container.textContent).toContain("Persistent conversation · Timeout: 600s");
    expect(instructions.value).toBe("Unfinished instructions");
    expect(mocks.saveSubagentAssignments).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, settings: { delegatable: false, persistent: true, timeoutSeconds: 600 } } });
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
    expect(mocks.success).toHaveBeenCalledWith("Assignment settings updated");
    expect([...container.querySelectorAll("button")].find((button) => button.textContent === "Run now")!.disabled).toBe(false);
    await click("Save instructions");
    expect(mocks.saveSubagentSection.mock.calls[0]![0].data.settings).toEqual({ instructions: "Unfinished instructions" });
    expect(container.textContent).toContain("Persistent conversation · Timeout: 600s");
  });
  it("discards assignment edits, validates timeouts, and retains failed saves for retry", async () => {
    await render(); await click("Edit assignments");
    const timeout = document.querySelector('dialog input[type="number"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(timeout, "29");
      timeout.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect([...document.querySelectorAll<HTMLButtonElement>("dialog button")].find((button) => button.textContent === "Apply")!.disabled).toBe(true);
    await click("Cancel");
    expect(container.textContent).toContain("Timeout: 900s");
    await click("Edit assignments");
    await act(async () => { (document.querySelector('dialog input[type="checkbox"]') as HTMLInputElement).click(); });
    await act(async () => { document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })); });
    expect(mocks.saveSubagentAssignments).not.toHaveBeenCalled();
    await click("Edit assignments");
    await act(async () => { (document.querySelector('dialog input[type="checkbox"]') as HTMLInputElement).click(); });
    mocks.saveSubagentAssignments.mockRejectedValueOnce(new Error("Stop the active assignment first"));
    await click("Apply");
    expect(document.querySelector("dialog [role=alert]")?.textContent).toContain("Stop the active assignment");
    expect(container.textContent).toContain("Orchestrator delegation allowed");
    await click("Apply");
    expect(document.querySelector("dialog")).toBeNull();
    expect(container.textContent).toContain("Orchestrator delegation disabled");
  });
  it("disables assignment editing for archived workers and hides it for orchestrators", async () => {
    await render({ ...definition, archived: true });
    expect([...container.querySelectorAll("button")].find((button) => button.textContent === "Edit assignments")!.matches(":disabled")).toBe(true);
    await render({ ...definition, kind: "orchestrator" });
    expect(container.textContent).not.toContain("Edit assignments");
  });
  it("shows a live countdown in the schedule and cleans up its timer", async () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
      const nextRunAt = Date.now() + 14 * 60_000;
      await render({ ...definition, schedule: "15m", nextRunAt });
      expect(container.textContent).toContain("Next run in 14 minutes");
      const time = container.querySelector("time")!;
      expect(time.dateTime).toBe("2026-01-01T00:14:00.000Z");
      expect(time.title).toBe(time.dateTime);
      await act(async () => { vi.advanceTimersByTime(60_000); });
      expect(container.textContent).toContain("Next run in 13 minutes");
      await act(async () => { vi.advanceTimersByTime(13 * 60_000); });
      expect(container.textContent).toContain("Next run due now");
      await act(async () => {
        HistorySource.instances.at(-1)!.dispatchEvent(new MessageEvent("runs", { data: JSON.stringify({ runs: [], pagination: { page: 1, pages: 1, total: 0, pageSize: 50 }, schedulerError: null, schedule: { nextRunAt: Date.now() + 15 * 60_000, updatedAt: definition.updatedAt } }) }));
      });
      expect(container.textContent).toContain("Next run in 15 minutes");
      await act(async () => root.render(<div />));
      expect(vi.getTimerCount()).toBe(0);
    } finally { vi.useRealTimers(); }
  });
  it("hides the next run when paused, archived, unscheduled or missing a timestamp", () => {
    for (const row of [
      { ...definition, schedule: "15m", nextRunAt: Date.now() + 60_000, enabled: false },
      { ...definition, schedule: "15m", nextRunAt: Date.now() + 60_000, archived: true },
      { ...definition, nextRunAt: Date.now() + 60_000 },
      { ...definition, schedule: "15m" },
    ]) {
      expect(renderToStaticMarkup(detailPage({ agentId: row.agentId, initial: initial(row), choices }))).not.toContain("Next run");
    }
  });
  it("updates the countdown when the saved next run changes", async () => {
    const onChange = vi.fn();
    await act(async () => root.render(<ScheduleConfig value={{ ...definition, schedule: "15m" }} nextRunAt={Date.now() + 14 * 60_000 + 1000} onChange={onChange} />));
    expect(container.textContent).toContain("Next run in 14 minutes");
    await act(async () => root.render(<ScheduleConfig value={{ ...definition, schedule: "15m" }} nextRunAt={Date.now() + 5 * 60_000 + 1000} onChange={onChange} />));
    expect(container.textContent).toContain("Next run in 5 minutes");
  });
  it("discards schedule edits on Escape", async () => {
    await render(); await click("Edit schedule");
    const schedule = document.querySelector("dialog select") as HTMLSelectElement;
    await act(async () => { schedule.value = "1m"; schedule.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(document.querySelector("dialog input")!.getAttribute("max")).toBe("59");
    await act(async () => { document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })); });
    expect(container.textContent).toContain("No schedule");
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
  });
  it("preserves manual assignment request IDs after uncertain network failures and shows server errors", async () => {
    await render(); mocks.runSubagent.mockRejectedValueOnce(new Error("Disconnected")).mockRejectedValueOnce(new Error("Worker already has an active assignment"));
    await click("Run now");
    const assignment = document.querySelector("dialog textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(assignment, "Research developments");
      assignment.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await click("Run assignment"); await click("Run assignment");
    expect(mocks.runSubagent).toHaveBeenCalledTimes(2); expect(mocks.runSubagent.mock.calls[0]![0]).toEqual(mocks.runSubagent.mock.calls[1]![0]);
    expect(document.querySelector("dialog [role=alert]")?.textContent).toContain("active assignment");
    expect(mocks.success).not.toHaveBeenCalled();
    expect(mocks.navigate).not.toHaveBeenCalled();
  });
  it("places run and archive actions in the title row and opens the assignment modal", async () => {
    await render();
    const header = container.querySelector("h2")!.closest("header")!;
    expect(header.textContent).toContain("Run now");
    const archive = header.querySelector<HTMLButtonElement>('[aria-label="Archive worker"]')!;
    expect(archive.textContent).toBe(""); expect(archive.className).toContain("text-red-300");
    expect(archive.querySelector("svg")).not.toBeNull();
    expect(container.querySelectorAll("textarea")).toHaveLength(1);
    await click("Run now");
    expect(document.querySelector("dialog")?.textContent).toContain("Run manually");
    const submit = [...document.querySelectorAll<HTMLButtonElement>("dialog button")].find((button) => button.textContent === "Run assignment")!;
    expect(submit.disabled).toBe(true);
    await click("Cancel");
    expect(document.querySelector("dialog")).toBeNull();
    await click("Run now");
    await act(async () => { document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })); });
    expect(document.querySelector("dialog")).toBeNull();
    expect(mocks.runSubagent).not.toHaveBeenCalled();
  });
  it("submits manual assignments with Enter but leaves Shift+Enter and composition alone", async () => {
    await render(); await click("Run now");
    const task = document.querySelector<HTMLTextAreaElement>("dialog textarea")!;
    const keydown = async (options: KeyboardEventInit = {}) => {
      const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, ...options });
      await act(async () => { task.dispatchEvent(event); });
      return event;
    };
    await keydown();
    expect(mocks.runSubagent).not.toHaveBeenCalled();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(task, "Research conditions");
      task.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect((await keydown({ shiftKey: true })).defaultPrevented).toBe(false);
    expect((await keydown({ isComposing: true })).defaultPrevented).toBe(false);
    expect(mocks.runSubagent).not.toHaveBeenCalled();
    mocks.runSubagent.mockResolvedValue({ id: "keyboard-run" });
    expect((await keydown()).defaultPrevented).toBe(true);
    expect(mocks.runSubagent).toHaveBeenCalledOnce();
    expect(mocks.runSubagent).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, task: "Research conditions", requestId: expect.any(String) } });
    expect(document.querySelector("dialog")).toBeNull();
  });
  it("redirects an accepted manual assignment to its run detail without saving settings", async () => {
    await render(); await editInstructions("Unsaved instructions");
    await click("Run now");
    const task = document.querySelector("dialog textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(task, "Research current conditions");
      task.dispatchEvent(new Event("input", { bubbles: true }));
    });
    mocks.runSubagent.mockResolvedValue({ id: "new-run-id" });
    await click("Run assignment");
    expect(document.querySelector("dialog")).toBeNull();
    expect(mocks.runSubagent).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, task: "Research current conditions", requestId: expect.any(String) } });
    expect(mocks.navigate).toHaveBeenCalledWith({ to: "/agents/$agentId/sub-agents/$subagentId/runs/$runId", params: { agentId: definition.agentId, subagentId: definition.id, runId: "new-run-id" } });
    expect(mocks.getSubagent).not.toHaveBeenCalled();
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
    expect(container.querySelector("textarea")!.value).toBe("Unsaved instructions");
    await click("Run now");
    expect((document.querySelector("dialog textarea") as HTMLTextAreaElement).value).toBe("");
  });
  it("does not resubmit an accepted assignment when the live history connection fails", async () => {
    await render(); await click("Run now");
    const task = document.querySelector("dialog textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(task, "Research");
      task.dispatchEvent(new Event("input", { bubbles: true }));
    });
    let resolve!: (value: unknown) => void;
    mocks.runSubagent.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    await click("Run assignment");
    expect((task as HTMLTextAreaElement).disabled).toBe(true);
    await click("Starting…"); await click("Cancel");
    expect(document.querySelector("dialog")).not.toBeNull();
    expect(mocks.runSubagent).toHaveBeenCalledOnce();
    await act(async () => { HistorySource.instances.at(-1)!.onerror?.(); resolve({ id: "accepted-run-id" }); });
    expect(document.querySelector("dialog")).toBeNull();
    expect(container.querySelector("[role=status]")?.textContent).toContain("Live updates interrupted");
    expect(mocks.runSubagent).toHaveBeenCalledOnce();
    expect(mocks.navigate).toHaveBeenCalledWith({ to: "/agents/$agentId/sub-agents/$subagentId/runs/$runId", params: { agentId: definition.agentId, subagentId: definition.id, runId: "accepted-run-id" } });
  });
  it("hides worker actions on the orchestrator, allows paused runs and disables archived workers", async () => {
    await render({ ...definition, kind: "orchestrator" });
    expect(container.querySelector('[aria-label="Archive worker"]')).toBeNull();
    expect(container.textContent).not.toContain("Run now");
    await render({ ...definition, enabled: false });
    expect([...container.querySelectorAll("button")].find((button) => button.textContent === "Run now")!.disabled).toBe(false);
    await click("Run now");
    expect(document.querySelector("dialog")).not.toBeNull();
    const task = document.querySelector("dialog textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(task, "Run paused worker once");
      task.dispatchEvent(new Event("input", { bubbles: true }));
    });
    mocks.runSubagent.mockResolvedValueOnce({ id: "paused-run" });
    await click("Run assignment");
    expect(mocks.runSubagent).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, task: "Run paused worker once", requestId: expect.any(String) } });
    expect((container.querySelector('[aria-label="Archive worker"]') as HTMLButtonElement).matches(":disabled")).toBe(false);
    await act(async () => root.render(detailPage({ key: "archived", agentId: definition.agentId, initial: initial({ ...definition, archived: true }), choices })));
    expect(container.querySelector('[aria-label="Archive worker"]')?.matches(":disabled")).toBe(true);
    expect([...container.querySelectorAll("button")].find((button) => button.textContent === "Run now")!.matches(":disabled")).toBe(true);
  });
  it("toggles enabled status beside the title immediately without saving instruction drafts", async () => {
    await render();
    await editInstructions("Unfinished instructions");
    const status = container.querySelector<HTMLButtonElement>('[aria-label="Pause sub-agent"]')!;
    expect(status.closest("nav")?.querySelector("h2")?.textContent).toBe("Research");
    expect(status.textContent).toBe("Enabled");
    expect(status.className).toContain("text-emerald-300");
    expect(status.getAttribute("aria-pressed")).toBe("true");
    await click("Pause sub-agent");
    expect(mocks.saveSubagentSection).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, section: "enabled", settings: { enabled: false } } });
    const paused = container.querySelector<HTMLButtonElement>('[aria-label="Enable sub-agent"]')!;
    expect(paused.textContent).toBe("Paused");
    expect(paused.className).toContain("text-amber-300");
    expect(paused.getAttribute("aria-pressed")).toBe("false");
    expect(container.querySelector("textarea")!.value).toBe("Unfinished instructions");
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
    await click("Edit assignments");
    expect(document.querySelector("dialog")?.textContent).not.toContain("Subagent enabled");
    expect(document.querySelectorAll('dialog input[type="checkbox"]')).toHaveLength(2);
    await click("Apply");
    expect(container.querySelector('[aria-label="Enable sub-agent"]')?.textContent).toBe("Paused");
    await click("Enable sub-agent");
    expect(container.querySelector('[aria-label="Pause sub-agent"]')?.textContent).toBe("Enabled");
    expect(mocks.success).toHaveBeenCalledWith("Sub-agent enabled");
    expect(mocks.success).toHaveBeenCalledWith("Sub-agent paused");
  });
  it("retains enabled status on failure and disables toggling during saves", async () => {
    await render();
    mocks.saveSubagentSection.mockRejectedValueOnce(new Error("Stop the active assignment first"));
    await click("Pause sub-agent");
    expect(container.querySelector("[role=alert]")?.textContent).toContain("Stop the active assignment");
    expect(container.querySelector('[aria-label="Pause sub-agent"]')?.textContent).toBe("Enabled");
    let resolve!: (value: unknown) => void;
    mocks.saveSubagentSection.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    await click("Pause sub-agent"); await click("Pause sub-agent");
    expect(mocks.saveSubagentSection).toHaveBeenCalledTimes(2);
    expect(container.querySelector('[aria-label="Pause sub-agent"]')?.matches(":disabled")).toBe(true);
    await act(async () => resolve({ ...definition, enabled: false }));
    expect(container.querySelector('[aria-label="Enable sub-agent"]')?.textContent).toBe("Paused");
    expect(container.querySelector("[role=alert]")).toBeNull();
  });
  it("protects orchestrator and archived status", async () => {
    await render({ ...definition, kind: "orchestrator" });
    expect(container.querySelector('[aria-label="Orchestrator always enabled"]')).toBeNull();
    expect(container.querySelector('[aria-label="Pause sub-agent"]')).toBeNull();
    expect(container.querySelector('[aria-label="Enable sub-agent"]')).toBeNull();
    expect(container.textContent).not.toContain("This is the existing main chat");
    expect(mocks.saveSubagentSection).not.toHaveBeenCalled();
    await render({ ...definition, archived: true, enabled: false });
    expect(container.querySelector('[aria-label="Enable sub-agent"]')?.matches(":disabled")).toBe(true);
    expect(mocks.saveSubagentSection).not.toHaveBeenCalled();
  });
  it("shows instruction actions only for changes and resets to the last saved value", async () => {
    await render();
    expect(container.textContent).not.toContain("Save instructions");
    expect([...container.querySelectorAll("button")].some((button) => button.textContent === "Reset")).toBe(false);
    await editInstructions("Draft instructions");
    expect(container.textContent).toContain("Save instructions");
    await click("Reset");
    expect(container.querySelector("textarea")!.value).toBe(definition.instructions);
    expect(mocks.saveSubagentSection).not.toHaveBeenCalled();
    await editInstructions("Saved instructions");
    await click("Save instructions");
    expect(mocks.saveSubagentSection).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id, section: "instructions", settings: { instructions: "Saved instructions" } } });
    expect(container.textContent).not.toContain("Save instructions");
    expect(mocks.success).toHaveBeenCalledWith("Instructions updated");
    await editInstructions("Another draft");
    await click("Reset");
    expect(container.querySelector("textarea")!.value).toBe("Saved instructions");
    await editInstructions("Another draft");
    await editInstructions("Saved instructions");
    expect(container.textContent).not.toContain("Save instructions");
  });
  it("retains instruction drafts on failure, rejects blank instructions and blocks duplicate saves", async () => {
    await render();
    await editInstructions("  ");
    expect([...container.querySelectorAll("button")].find((button) => button.textContent === "Save instructions")!.disabled).toBe(true);
    await editInstructions("Retry this draft");
    mocks.saveSubagentSection.mockRejectedValueOnce(new Error("Stop the active assignment first"));
    await click("Save instructions");
    expect(container.querySelector("[role=alert]")?.textContent).toContain("Stop the active assignment");
    expect(container.querySelector("textarea")!.value).toBe("Retry this draft");
    let resolve!: (value: unknown) => void;
    mocks.saveSubagentSection.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    await click("Save instructions"); await click("Save instructions");
    expect(mocks.saveSubagentSection).toHaveBeenCalledTimes(2);
    expect([...container.querySelectorAll("button")].find((button) => button.textContent === "Reset")!.matches(":disabled")).toBe(true);
    await act(async () => resolve({ ...definition, instructions: "Retry this draft" }));
    expect(container.textContent).not.toContain("Save instructions");
    expect(container.querySelector("[role=alert]")).toBeNull();
  });
  it("keeps model and schedule modals open when an automatic save fails", async () => {
    await render();
    await editInstructions("Unsaved instructions");
    for (const trigger of ["Choose model", "Edit schedule"]) {
      await click(trigger);
      mocks.saveSubagentSection.mockRejectedValueOnce(new Error("Unable to update now"));
      await click("Apply");
      expect(document.querySelector("dialog [role=alert]")?.textContent).toContain("Unable to update now");
      await click("Apply");
      expect(document.querySelector("dialog")).toBeNull();
      expect(container.querySelector("textarea")!.value).toBe("Unsaved instructions");
      expect(container.textContent).toContain("Save instructions");
    }
  });
  it("links back through the header breadcrumb without page-wide save or cancel controls", async () => {
    await render();
    const breadcrumb = container.querySelector('nav[aria-label="Sub-agent breadcrumb"]')!;
    expect(breadcrumb.querySelector("a")?.getAttribute("href")).toBe("/agents/agent-fixture/sub-agents");
    expect(breadcrumb.textContent).toBe("← Sub-agents/ResearchEnabledFind evidence");
    const edit = breadcrumb.querySelector('[aria-label="Edit name and description"]')!;
    expect(edit.textContent).toBe("");
    expect(edit.querySelector("svg")).not.toBeNull();
    expect(edit.previousElementSibling?.textContent).toBe("Find evidence");
    expect(container.textContent).not.toContain("Save settings");
    expect([...container.querySelectorAll("a, button")].some((element) => element.textContent === "Cancel")).toBe(false);
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
  });
});
