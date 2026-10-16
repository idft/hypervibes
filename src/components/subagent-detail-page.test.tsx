// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { SubagentRow } from "../db/subagent-schema.js";
import { mainToolNames } from "../agents/subagent-input.js";
const mocks = vi.hoisted(() => ({ invalidate: vi.fn(), navigate: vi.fn(), success: vi.fn(), saveSubagent: vi.fn(), getSubagent: vi.fn(), runSubagent: vi.fn(), archiveSubagent: vi.fn(), stopSubagentRun: vi.fn() }));
vi.mock("../web/subagents.js", () => mocks);
vi.mock("sonner", () => ({ toast: { success: mocks.success } }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => ({ invalidate: mocks.invalidate, navigate: mocks.navigate }), Link: ({ to, params, children, ...props }: { to: string; params: Record<string, string>; children: React.ReactNode }) => <a {...props} href={to.replace(/\$(\w+)/g, (_match, key: string) => encodeURIComponent(params[key]!))}>{children}</a> }));
import { SubagentDetailPage } from "./subagent-detail-page.js";
import { SubagentsPage } from "./subagents-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const definition: SubagentRow = { id: "worker:fixture", agentId: "agent-fixture", kind: "worker", name: "Research", description: "Find evidence", instructions: "Research carefully", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [], enabled: true, archived: false, delegatable: true, persistent: false, conversationId: null, schedule: "", delaySeconds: 30, nextRunAt: null, completion: "wake", timeoutSeconds: 900, createdAt: 1, updatedAt: 1 };
const choices = [{ provider: "faux", modelId: "faux-1", name: "Faux", thinkingLevels: ["high"] }];
describe("subagent pages", () => {
  let root: Root, container: HTMLDivElement;
  const initial = (row = definition) => ({ definition: row, runs: [], schedulerError: null });
  const render = async (row = definition) => act(async () => root.render(<SubagentDetailPage agentId={row.agentId} initial={initial(row)} choices={choices} />));
  const click = async (text: string) => act(async () => { [...container.querySelectorAll("button")].find((button) => button.textContent === text)!.click(); });
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
  beforeEach(() => { vi.resetAllMocks(); container = document.createElement("div"); document.body.append(container); root = createRoot(container); mocks.getSubagent.mockResolvedValue({ ok: true, data: initial() }); });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); });
  it("lists the protected orchestrator and links workers to detail pages", () => {
    const html = renderToStaticMarkup(<SubagentsPage agentId={definition.agentId} definitions={[{ ...definition, id: "main:fixture", kind: "orchestrator", name: "Orchestrator" }, definition]} />);
    expect(html).toContain("Main conversation · Protected"); expect(html).toContain("Create subagent");
    expect(html).toContain("/agents/agent-fixture/sub-agents/worker%3Afixture"); expect(html).toContain("Fresh context per assignment");
  });
  it("hides archived workers by default and lets users show or hide them", async () => {
    const archived = { ...definition, id: "worker:archived", name: "Old research", archived: true, enabled: false };
    await act(async () => root.render(<SubagentsPage agentId={definition.agentId} definitions={[definition, archived]} />));
    expect(container.textContent).not.toContain("Configure your orchestrator");
    expect(container.textContent).not.toContain("Old research");
    expect(container.querySelector("button")?.getAttribute("aria-pressed")).toBe("false");
    await click("Show archived (1)");
    expect(container.textContent).toContain("Old research");
    expect(container.querySelector("button")?.getAttribute("aria-pressed")).toBe("true");
    await click("Hide archived");
    expect(container.textContent).not.toContain("Old research");
    expect(container.textContent).toContain("Research");
  });
  it("redirects to the refreshed index after archiving", async () => {
    await render(); const browserConfirm = vi.spyOn(window, "confirm");
    mocks.archiveSubagent.mockResolvedValue({ ok: true });
    await click("Archive worker");
    const dialog = container.querySelector("dialog")!;
    expect(dialog.open).toBe(true); expect(dialog.textContent).toContain("Research");
    expect(dialog.getAttribute("aria-labelledby")).toBeTruthy();
    expect(mocks.archiveSubagent).not.toHaveBeenCalled(); expect(browserConfirm).not.toHaveBeenCalled();
    await click("Archive sub-agent");
    expect(mocks.archiveSubagent).toHaveBeenCalledWith({ data: { agentId: definition.agentId, id: definition.id } });
    expect(mocks.invalidate).toHaveBeenCalledOnce();
    expect(mocks.navigate).toHaveBeenCalledWith({ to: "/agents/$agentId/sub-agents", params: { agentId: definition.agentId } });
    expect(mocks.getSubagent).not.toHaveBeenCalled();
    expect(mocks.success).toHaveBeenCalledWith("Sub-agent archived", { description: "History is preserved." });
  });
  it("stays on the detail page when archiving fails", async () => {
    await render();
    mocks.archiveSubagent.mockResolvedValue({ ok: false, error: "Stop the active assignment first" });
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
  it("edits the orchestrator model, thinking and tools without offering deletion or schedules", async () => {
    const main = { ...definition, id: "main:fixture", name: "Orchestrator", kind: "orchestrator" as const, tools: mainToolNames };
    await render(main); mocks.saveSubagent.mockResolvedValue({ ok: true, data: { ...main, thinkingLevel: "high" } });
    expect(container.textContent).toContain("cannot be disabled, archived, or deleted"); expect(container.textContent).not.toContain("Archive worker"); expect(container.textContent).not.toContain("UTC candle close");
    const thinking = [...container.querySelectorAll("select")].find((select) => [...select.options].some((option) => option.value === "high"))!;
    await act(async () => { thinking.value = "high"; thinking.dispatchEvent(new Event("change", { bubbles: true })); });
    await click("Save settings");
    expect(mocks.saveSubagent).toHaveBeenCalledWith({ data: expect.objectContaining({ id: main.id, settings: expect.objectContaining({ thinkingLevel: "high", tools: mainToolNames }) }) });
    expect(mocks.success).toHaveBeenCalledWith("Sub-agent updated");
    expect(container.textContent).not.toContain("Settings saved");
  });
  it("offers candle-close scheduling, lifetime and completion choices without recursive worker tools", async () => {
    await render(); expect(container.textContent).toContain("Allow orchestrator delegation"); expect(container.textContent).toContain("Reuse a persistent conversation"); expect(container.textContent).not.toContain("delegate_subagent");
    const schedule = [...container.querySelectorAll("select")].find((select) => [...select.options].some((option) => option.value === "1h"))!;
    await act(async () => { schedule.value = "1h"; schedule.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(container.textContent).toContain("Delay after close"); expect(container.textContent).not.toContain("Scheduled assignment");
    expect(container.textContent).not.toContain("UTC-aligned"); expect(container.textContent).not.toContain("Delegated assignments");
    expect(container.textContent).toContain("Save only"); expect(container.textContent).toContain("without a model call"); expect(container.textContent).toContain("Notify and wake");
    mocks.saveSubagent.mockResolvedValue({ ok: true, data: { ...definition, schedule: "1h" } });
    await click("Save settings"); expect(mocks.saveSubagent.mock.calls[0]![0].data.settings.schedule).toBe("1h");
  });
  it("preserves manual assignment request IDs after uncertain network failures and shows server errors", async () => {
    await render(); mocks.runSubagent.mockRejectedValueOnce(new Error("Disconnected")).mockResolvedValueOnce({ ok: false, error: "Worker already has an active assignment" });
    const assignment = container.querySelectorAll("textarea")[1]!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(assignment, "Research developments");
      assignment.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await click("Run now"); await click("Run now");
    expect(mocks.runSubagent).toHaveBeenCalledTimes(2); expect(mocks.runSubagent.mock.calls[0]![0]).toEqual(mocks.runSubagent.mock.calls[1]![0]);
    expect(container.querySelector("[role=alert]")?.textContent).toContain("active assignment");
    expect(mocks.success).not.toHaveBeenCalled();
  });
  it("offers Cancel beside Save settings without submitting changes", async () => {
    await render();
    const cancel = [...container.querySelectorAll("a")].find((link) => link.textContent === "Cancel")!;
    expect(cancel.getAttribute("href")).toBe("/agents/agent-fixture/sub-agents");
    expect(cancel.parentElement?.querySelector("button")?.textContent).toBe("Save settings");
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
  });
  it("creates a definition without running it", async () => {
    await act(async () => root.render(<SubagentDetailPage agentId={definition.agentId} choices={choices} />));
    mocks.saveSubagent.mockResolvedValue({ ok: true, data: definition });
    await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(mocks.saveSubagent.mock.calls[0]![0].data.id).toBe(""); expect(mocks.runSubagent).not.toHaveBeenCalled(); expect(container.textContent).toContain("Run manually");
    expect(mocks.success).toHaveBeenCalledWith("Sub-agent created");
    expect(container.querySelector("[role=status]")).toBeNull();
  });
});
