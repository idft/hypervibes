// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { subagentTemplates, templateDraft } from "../agents/subagent-templates.js";
const mocks = vi.hoisted(() => ({ navigate: vi.fn(), saveSubagent: vi.fn(), success: vi.fn() }));
vi.mock("../web/subagents.js", () => ({ saveSubagent: mocks.saveSubagent }));
vi.mock("sonner", () => ({ toast: { success: mocks.success } }));
vi.mock("@tanstack/react-router", () => ({ useRouter: () => ({ navigate: mocks.navigate }), Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a> }));
import { NewSubagentPage } from "./new-subagent-page.js";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const model = { provider: "parent-provider", modelId: "parent-model", thinkingLevel: "high" };
const choices = [model, { provider: "other", modelId: "other-model", thinkingLevel: "" }].map((entry) => ({ ...entry, name: entry.modelId, providerName: entry.provider, thinkingLevels: ["high"], contextWindow: 200_000, inputCost: 3, outputCost: 15, reasoning: true }));
describe("new sub-agent", () => {
  let root: Root, container: HTMLDivElement;
  const submit = () => act(async () => { container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
  const name = () => container.querySelector("input")!;
  const description = () => container.querySelector("textarea")!;
  const input = (element: HTMLInputElement | HTMLTextAreaElement, value: string) => act(async () => {
    Object.getOwnPropertyDescriptor(element instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const click = (text: string) => act(async () => { [...document.querySelectorAll<HTMLButtonElement>("button")].find((element) => element.textContent === text || element.getAttribute("aria-label") === text)!.click(); });
  const select = (id: string) => click(subagentTemplates.find((entry) => entry.id === id)?.name ?? "Custom");
  const originalDialogMethods = Object.fromEntries(["showModal", "close"].map((key) => [key, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, key)]));
  beforeAll(() => {
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
  beforeEach(async () => {
    vi.resetAllMocks();
    mocks.saveSubagent.mockResolvedValue({ ok: true, data: { id: "worker:new" } });
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<NewSubagentPage agentId="agent-1" model={model} choices={choices} />));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  it("lists templates on the left and fills editable identity fields on selection, clearing them for Custom", async () => {
    expect(container.querySelectorAll("input, textarea, select")).toHaveLength(2);
    expect(container.textContent).not.toContain("Edit tools");
    expect([...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Create sub-agent")!.disabled).toBe(true);
    for (const template of subagentTemplates) {
      await select(template.id);
      expect(name().value).toBe(template.name);
      expect(description().value).toBe(template.description);
      expect([...container.querySelectorAll("button")].find((button) => button.textContent === template.name)?.getAttribute("aria-pressed")).toBe("true");
      await input(name(), "My worker"); await input(description(), "Custom description");
      expect(name().value).toBe("My worker"); expect(description().value).toBe("Custom description");
    }
    await select("");
    expect(name().value).toBe(""); expect(description().value).toBe("");
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
  });
  it.each(["", "basic-analysis", "basic-trading", "basic-review"])("creates %s paused with the parent model and navigates to its permanent detail URL", async (templateId) => {
    await select(templateId); await input(name(), "My worker"); await submit();
    expect(mocks.saveSubagent).toHaveBeenCalledOnce();
    const data = mocks.saveSubagent.mock.calls[0]![0].data;
    expect(data).toMatchObject({ agentId: "agent-1", id: "", requestId: expect.any(String), templateId, settings: { ...model, name: "My worker", enabled: false } });
    if (templateId) expect(data.settings).toEqual({ ...templateDraft(templateId, model), name: "My worker", enabled: false });
    else expect(data.settings).toMatchObject({ tools: [], schedule: "", persistent: false });
    expect(mocks.navigate).toHaveBeenCalledWith({ to: "/agents/$agentId/sub-agents/$subagentId", params: { agentId: "agent-1", subagentId: "worker:new" } });
    expect(mocks.success).toHaveBeenCalledWith("Sub-agent created paused", { description: "Review the configuration, then enable when ready." });
  });
  it("defaults to the root model and applies a different model before creation without template changes resetting it", async () => {
    expect(container.querySelector('[aria-label="Choose model"]')?.textContent).toContain("parent-provider / parent-model");
    expect(container.querySelector('[aria-label="Choose model"]')?.textContent).toContain("Thinking: high");
    await click("Choose model");
    await act(async () => { (document.querySelectorAll('dialog input[type="radio"]')[1] as HTMLInputElement).click(); });
    await click("Apply");
    await select("basic-trading");
    expect(container.querySelector('[aria-label="Choose model"]')?.textContent).toContain("other / other-model");
    await submit();
    expect(mocks.saveSubagent.mock.calls[0]![0].data.settings).toMatchObject({ name: subagentTemplates.find((entry) => entry.id === "basic-trading")!.name, provider: "other", modelId: "other-model", thinkingLevel: "", enabled: false });
  });
  it.each(["off", "high"])("uses Default when the inherited thinking level %s is unsupported, without requiring the model picker to be opened", async (thinkingLevel) => {
    await act(async () => root.render(<NewSubagentPage key={thinkingLevel} agentId="agent-1" model={{ ...model, thinkingLevel }} choices={[{ ...choices[0]!, thinkingLevels: [], reasoning: false }]} />));
    expect(container.querySelector('[aria-label="Choose model"]')?.textContent).toContain("Thinking: Default");
    await select("basic-analysis"); await submit();
    expect(mocks.saveSubagent.mock.calls[0]![0].data.settings).toMatchObject({ provider: model.provider, modelId: model.modelId, thinkingLevel: "", enabled: false });
    expect(mocks.navigate).toHaveBeenCalledOnce();
  });
  it("discards unconfirmed model changes", async () => {
    await click("Choose model");
    await act(async () => { (document.querySelectorAll('dialog input[type="radio"]')[1] as HTMLInputElement).click(); });
    await click("Cancel");
    await input(name(), "Worker"); await submit();
    expect(mocks.saveSubagent.mock.calls[0]![0].data.settings).toMatchObject(model);
  });
  it("requires an available model rather than silently replacing an unavailable root model", async () => {
    await act(async () => root.render(<NewSubagentPage key="unavailable" agentId="agent-1" model={model} choices={[choices[1]!]} />));
    expect(container.querySelector('[role="status"]')?.textContent).toContain("is unavailable");
    await input(name(), "Worker"); await submit();
    expect(mocks.saveSubagent).not.toHaveBeenCalled();
    await click("Choose model");
    await act(async () => { (document.querySelector('dialog input[type="radio"]') as HTMLInputElement).click(); });
    await click("Apply"); await submit();
    expect(mocks.saveSubagent.mock.calls[0]![0].data.settings).toMatchObject({ provider: "other", modelId: "other-model" });
  });
  it("rejects blank names and lets users correct server validation failures", async () => {
    await input(name(), "   "); await submit(); expect(mocks.saveSubagent).not.toHaveBeenCalled();
    await input(name(), "Worker");
    mocks.saveSubagent.mockResolvedValueOnce({ ok: false, error: "Select a model." });
    await submit();
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Select a model.");
    expect(name().matches(":disabled")).toBe(false);
    expect(mocks.navigate).not.toHaveBeenCalled();
    await input(name(), "Corrected"); await submit();
    expect(mocks.saveSubagent.mock.calls[1]![0].data.settings.name).toBe("Corrected");
  });
  it("blocks duplicate submissions and reuses identical settings after uncertain network failures", async () => {
    await input(name(), "Worker");
    let reject!: (error: Error) => void;
    mocks.saveSubagent.mockImplementationOnce(() => new Promise((_resolve, fail) => { reject = fail; }));
    await submit(); await submit();
    expect(mocks.saveSubagent).toHaveBeenCalledOnce();
    await act(async () => reject(new Error("Disconnected")));
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("retry safely");
    expect(name().matches(":disabled")).toBe(true);
    expect(container.querySelector('[aria-label="Choose model"]')?.matches(":disabled")).toBe(true);
    expect([...container.querySelectorAll('[aria-pressed]')].every((button) => button.matches(":disabled"))).toBe(true);
    await submit();
    expect(mocks.saveSubagent.mock.calls[1]![0]).toEqual(mocks.saveSubagent.mock.calls[0]![0]);
  });
  it("retries navigation without creating again after a successful save", async () => {
    await input(name(), "Worker"); mocks.navigate.mockRejectedValueOnce(new Error("Navigation failed"));
    await submit(); await submit();
    expect(mocks.saveSubagent).toHaveBeenCalledOnce(); expect(mocks.navigate).toHaveBeenCalledTimes(2);
  });
});
