// @vitest-environment jsdom
import { act } from "react";
import { type QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createTestQueryClient } from "../test/query.js";
import { chartLayoutsQueryOptions } from "../web/chart-queries.js";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentChat } from "../agents/agent-service.js";
import { chartEdit, newDrawing, type ChartLayout } from "../charts/contracts.js";

const mocks = vi.hoisted(() => ({ create: vi.fn(), change: vi.fn(), remove: vi.fn(), list: vi.fn(), capture: vi.fn(), chartProps: {} as Record<string, unknown>, chatProps: {} as Record<string, unknown> }));
vi.mock("../web/charts.js", () => ({ createChartLayout: mocks.create, changeChartLayout: mocks.change, deleteChartLayout: mocks.remove, getChartLayouts: mocks.list }));
vi.mock("./market-chart.js", async () => {
  const { useImperativeHandle } = await import("react");
  const { createPortal } = await import("react-dom");
  return { MarketChart: (props: { ref: React.Ref<unknown>; tool: string; statusHost: HTMLElement | null; onSave: (value: unknown) => void; onSelect: (id: string) => void }) => {
    mocks.chartProps = props;
    useImperativeHandle(props.ref, () => ({ capture: mocks.capture }));
    return <>{props.statusHost && createPortal(<span role="status" aria-label="Chart connection">Connected</span>, props.statusHost)}<div aria-label="Market fixture"><button onClick={() => { const d = newDrawing("horizontal", "BTC", [{ time: 3600, price: 100 }]); props.onSave(d); props.onSelect(d.id); }}>Place fixture level</button></div></>;
  } };
});
vi.mock("./agent-chat-page.js", () => ({ AgentChatPage: (props: Record<string, unknown>) => { mocks.chatProps = props; return <div aria-label="Root chat fixture">Main conversation</div>; } }));
import { AgentChartPage } from "./agent-chart-page.js";
const instruments = ["BTC", "ETH"];
const chat = { agent: { id: "fixture" } } as AgentChat;
class LayoutSource extends EventTarget {
  static instances: LayoutSource[] = [];
  close = vi.fn();
  constructor(readonly url: string) { super(); LayoutSource.instances.push(this); }
  emit(layouts: ChartLayout[]) { this.dispatchEvent(new MessageEvent("chart-layouts", { data: JSON.stringify(layouts) })); }
}

describe("chart workspace", () => {
  let root: Root, container: HTMLDivElement, layout: ChartLayout, initial: ChartLayout[], client: QueryClient;
  let observedResize: ResizeObserverCallback;
  beforeEach(() => {
    client = createTestQueryClient();
    localStorage.clear(); vi.resetAllMocks(); LayoutSource.instances = []; vi.stubGlobal("EventSource", LayoutSource);
    Element.prototype.scrollIntoView = vi.fn();
    vi.stubGlobal("ResizeObserver", class { constructor(callback: ResizeObserverCallback) { observedResize = callback; } observe() {} disconnect() {} });
    const captured = new Set<number>();
    HTMLElement.prototype.setPointerCapture = vi.fn((id: number) => { captured.add(id); });
    HTMLElement.prototype.hasPointerCapture = vi.fn((id: number) => captured.has(id));
    HTMLElement.prototype.releasePointerCapture = vi.fn((id: number) => { captured.delete(id); });
    layout = { id: crypto.randomUUID(), revision: 0, document: { name: "My chart", coin: "BTC", interval: "1h", drawings: [], notes: "", hideDrawings: false, magnet: true }, updatedAt: 0, canUndo: false, canRedo: false, lastChange: "Created" }; initial = [layout];
    mocks.list.mockImplementation(async () => [layout]);
    mocks.change.mockImplementation(async ({ data }: { data: { action: string; edit: unknown } }) => {
      const edit = data.action === "edit" ? chartEdit(data.edit) : { label: data.action, upsert: [], remove: [] };
      const drawings = new Map(layout.document.drawings.map((d) => [d.id, d]));
      for (const id of edit.remove ?? []) drawings.delete(id);
      for (const d of edit.upsert ?? []) drawings.set(d.id, d);
      layout = { ...layout, revision: layout.revision + 1, canUndo: true, document: { ...layout.document, ...edit.settings, drawings: [...drawings.values()] } };
      return layout;
    });
    mocks.create.mockImplementation(async ({ data }: { data: { layoutId: string; settings: { name: string; coin?: string } } }) => ({ ...layout, id: data.layoutId, document: { ...layout.document, ...data.settings } }));
    mocks.remove.mockResolvedValue(undefined);
    mocks.capture.mockResolvedValue({ layoutId: layout.id, revision: 0 });
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
  async function render() {
    client.setQueryData(chartLayoutsQueryOptions("fixture").queryKey, initial);
    await act(async () => root.render(<QueryClientProvider client={client}><AgentChartPage id="fixture" instruments={instruments} initialChat={chat} choices={[]} /></QueryClientProvider>));
    await act(async () => LayoutSource.instances.at(-1)!.emit(initial));
  }
  async function click(label: string) { await act(async () => {
    const target = [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label || b.getAttribute("aria-label") === label);
    expect(target, label).toBeDefined(); target!.click();
  }); }
  async function select(label: string, value: string) {
    if (label === "Instrument") {
      await click("Instrument");
      await act(async () => document.querySelector<HTMLInputElement>(`dialog input[type=radio][aria-label='${value}']`)!.click());
      await click("Apply");
      return;
    }
    await act(async () => { const e = container.querySelector<HTMLSelectElement>(`[aria-label="${label}"]`)!; e.value = value; e.dispatchEvent(new Event("change", { bubbles: true })); });
  }
  it("uses one non-wrapping toolbar in the requested order, with accessible icon-only toggles", async () => {
    await render();
    const toolbar = container.querySelector("[role=toolbar][aria-label='Chart controls']")!;
    expect([...toolbar.children].map((item) => item.getAttribute("aria-label"))).toEqual(["Layout", "Layout settings", "Instrument", "Timeframe", "Account overlays", "Magnet", "Hide drawings", "Show annotation tools", "Lock objects", "Hide chat", "Chart status"]);
    expect(toolbar.classList.contains("flex-nowrap")).toBe(true);
    expect(toolbar.querySelector("label")).toBeNull();
    const status = toolbar.querySelector("[aria-label='Chart status']")!;
    expect(status.classList.contains("h-8")).toBe(true);
    expect(status.querySelector("[aria-label='Chart connection']")!.textContent).toBe("Connected");
    expect(mocks.chartProps.statusHost).toBe(status);
    expect(container.querySelector("[aria-label='Chart canvas'] [aria-label='Chart connection']")).toBeNull();
    for (const name of ["Layout settings", "Account overlays", "Magnet", "Hide drawings", "Show annotation tools", "Lock objects", "Hide chat"]) {
      const icon = toolbar.querySelector(`[aria-label='${name}']`)!;
      expect(icon.querySelector("svg")).not.toBeNull(); expect(icon.textContent).toBe("");
    }
    expect([...container.querySelectorAll("button")].some((b) => ["Undo", "Redo", "Objects & notes", "New", "Duplicate", "Rename", "Delete layout"].includes(b.textContent ?? ""))).toBe(false);
    expect(document.querySelector("[role=dialog][aria-label='Layout settings']")).toBeNull();
  });
  it("persists read-only overlay visibility outside layouts and their undo history", async () => {
    await render();
    expect(mocks.chartProps.overlayVisibility).toEqual({ trades: true, position: true, orders: true });
    await click("Account overlays");
    const checkbox = document.querySelector<HTMLInputElement>("dialog input[type=checkbox]")!;
    await act(async () => checkbox.click());
    await click("Cancel");
    expect(mocks.chartProps.overlayVisibility).toEqual({ trades: true, position: true, orders: true });
    await click("Account overlays");
    await act(async () => document.querySelector<HTMLInputElement>("dialog input[type=checkbox]")!.click());
    await click("Apply");
    expect(mocks.chartProps.overlayVisibility).toEqual({ trades: false, position: true, orders: true });
    expect(JSON.parse(localStorage.getItem("hypervibes:chart-overlays:v1:fixture")!)).toEqual(mocks.chartProps.overlayVisibility);
    expect(mocks.change).not.toHaveBeenCalled(); expect(mocks.create).not.toHaveBeenCalled();
    await act(async () => { root.unmount(); root = createRoot(container); });
    await render();
    expect(mocks.chartProps.overlayVisibility).toEqual({ trades: false, position: true, orders: true });
  });
  it("opens a searchable single-instrument modal with logos and avoids writes on cancel or unchanged selection", async () => {
    await render();
    const trigger = container.querySelector<HTMLButtonElement>("button[aria-label=Instrument]")!;
    expect(container.querySelector("select[aria-label=Instrument]")).toBeNull();
    expect(trigger.querySelector("img")!.getAttribute("src")).toBe("/currency/BTC.svg");
    await click("Instrument");
    expect(document.querySelector("dialog input[type=search]")).not.toBeNull();
    expect(document.querySelectorAll("dialog input[type=radio]")).toHaveLength(2);
    expect(document.querySelectorAll("dialog input[type=checkbox]")).toHaveLength(0);
    await act(async () => document.querySelector<HTMLInputElement>("dialog input[aria-label=ETH]")!.click());
    await click("Cancel");
    expect(trigger.textContent).toContain("BTC"); expect(mocks.change).not.toHaveBeenCalled();
    await click("Instrument"); await click("Apply");
    expect(document.querySelector("dialog[open]")).toBeNull(); expect(mocks.change).not.toHaveBeenCalled();
  });
  it("opens layout actions in the gear popup and dismisses it with Escape or an outside click", async () => {
    await render(); await click("Layout settings");
    const popup = () => document.querySelector("[role=dialog][aria-label='Layout settings']");
    expect(popup()!.textContent).toContain("NewDuplicateRenameDeleteLayout notes");
    expect(container.contains(popup())).toBe(false); // Portal is outside the scrolling toolbar.
    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(popup()).toBeNull();
    expect(document.activeElement).toBe(container.querySelector("[aria-label='Layout settings']"));
    await click("Layout settings");
    await act(async () => document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })));
    expect(popup()).toBeNull();
  });
  it("keeps new, duplicate, rename and confirmed deletion available through layout settings", async () => {
    await render();
    for (const [label, title] of [["New", "New layout"], ["Duplicate", "Duplicate layout"], ["Rename", "Rename layout"]]) {
      await click("Layout settings"); await click(label!);
      expect(document.querySelector("dialog[open]")!.textContent).toContain(title);
      await click("Cancel");
    }
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValueOnce(true);
    await click("Layout settings"); await click("Delete layout");
    expect(confirm).toHaveBeenCalled(); expect(mocks.remove).not.toHaveBeenCalled();
    await click("Layout settings"); await click("Delete layout");
    expect(mocks.remove).toHaveBeenCalledWith({ data: { id: "fixture", layoutId: layout.id, revision: 0 } });
  });
  it("applies rename, duplicate and new layout actions from the popup", async () => {
    await render();
    const name = async (value: string) => {
      const input = document.querySelector<HTMLInputElement>("dialog input")!;
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    };
    await click("Layout settings"); await click("Rename"); await name("Working"); await click("Apply");
    expect(mocks.change).toHaveBeenLastCalledWith({ data: expect.objectContaining({ edit: { label: "Rename layout", settings: { name: "Working" } } }) });
    expect(container.querySelector<HTMLSelectElement>("[aria-label=Layout]")!.selectedOptions[0]!.textContent).toBe("Working");
    await click("Layout settings"); await click("Duplicate"); await click("Apply");
    expect(mocks.create).toHaveBeenLastCalledWith({ data: expect.objectContaining({ sourceId: layout.id, settings: { name: "Working copy" } }) });
    await click("Layout settings"); await click("New"); await name("New idea"); await click("Apply");
    expect(mocks.create).toHaveBeenLastCalledWith({ data: expect.objectContaining({ settings: { name: "New idea", coin: "BTC", interval: "1h" } }) });
    expect(mocks.create.mock.calls.at(-1)![0].data).not.toHaveProperty("sourceId");
  });
  it("edits layout-local notes through the gear popup", async () => {
    await render(); await click("Layout settings"); await click("Layout notes");
    const dialog = document.querySelector("dialog[open]")!;
    const textarea = dialog.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "Wait for support");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await click("Save notes");
    expect(mocks.change).toHaveBeenCalledWith({ data: expect.objectContaining({ edit: { label: "Update layout notes", settings: { notes: "Wait for support" } } }) });
    expect(document.querySelector("dialog[open]")).toBeNull();
  });
  it("locks and unlocks all objects for the current instrument without modifying other instruments", async () => {
    const a = newDrawing("horizontal", "BTC", [{ time: 3600, price: 100 }]);
    const b = { ...a, id: crypto.randomUUID(), locked: true }, other = { ...a, id: crypto.randomUUID(), coin: "ETH" };
    layout = { ...layout, document: { ...layout.document, drawings: [a, b, other] } }; initial = [layout];
    await render(); await click("Lock objects");
    expect(mocks.change).toHaveBeenLastCalledWith({ data: expect.objectContaining({ edit: { label: "Lock drawings", upsert: [{ ...a, locked: true }] } }) });
    expect(container.querySelector("[aria-label='Unlock objects']")!.getAttribute("aria-pressed")).toBe("true");
    await click("Unlock objects");
    expect(mocks.change).toHaveBeenLastCalledWith({ data: expect.objectContaining({ edit: { label: "Unlock drawings", upsert: [{ ...a, locked: false }, { ...b, locked: false }] } }) });
    expect(layout.document.drawings.find((d) => d.id === other.id)).toEqual(other);
  });
  it("retains undo and redo shortcuts without visible undo/redo buttons, while respecting text editing", async () => {
    layout = { ...layout, canUndo: true, canRedo: true }; initial = [layout]; await render();
    const canvas = container.querySelector("[aria-label='Chart canvas']")!;
    for (const [key, shiftKey, action] of [["z", false, "undo"], ["z", true, "redo"], ["y", false, "redo"]] as const) {
      await act(async () => canvas.dispatchEvent(new KeyboardEvent("keydown", { key, ctrlKey: true, shiftKey, bubbles: true, cancelable: true })));
      expect(mocks.change).toHaveBeenLastCalledWith({ data: expect.objectContaining({ action }) });
    }
    await click("Layout settings"); await click("Layout notes");
    const calls = mocks.change.mock.calls.length;
    await act(async () => document.querySelector("dialog textarea")!.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, bubbles: true })));
    expect(mocks.change).toHaveBeenCalledTimes(calls);
  });
  it("resizes the right-side chat with pointer coordinates and persists its final width", async () => {
    await render();
    const divider = container.querySelector("[role=separator]")!;
    const pointer = (type: string, x: number) => divider.dispatchEvent(Object.assign(new MouseEvent(type, { button: 0, clientX: x, bubbles: true, cancelable: true }), { pointerId: 1 }));
    await act(async () => { pointer("pointerdown", 600); pointer("pointermove", 550); pointer("pointerup", 530); });
    expect(divider.getAttribute("aria-valuenow")).toBe("430");
    expect(JSON.parse(localStorage.getItem("hypervibes:chart-panel:fixture")!)).toEqual({ open: true, width: 430 });
    expect(container.querySelector("[aria-label='Chart and chat workspace']")!.lastElementChild!.getAttribute("aria-label")).toBe("Chart chat panel");
  });
  it("cancels a chat resize without persisting its transient width", async () => {
    await render();
    const divider = container.querySelector("[role=separator]")!;
    const pointer = (type: string, x: number) => divider.dispatchEvent(Object.assign(new MouseEvent(type, { button: 0, clientX: x, bubbles: true, cancelable: true }), { pointerId: 1 }));
    await act(async () => { pointer("pointerdown", 600); pointer("pointermove", 550); });
    expect(divider.getAttribute("aria-valuenow")).toBe("410");
    await act(async () => { pointer("pointercancel", 550); });
    expect(divider.getAttribute("aria-valuenow")).toBe("360");
    expect(localStorage.getItem("hypervibes:chart-panel:fixture")).toBeNull();
  });
  it("restores width preferences and clamps the chat when the workspace becomes narrower", async () => {
    localStorage.setItem("hypervibes:chart-panel:fixture", JSON.stringify({ open: true, width: 500 }));
    await render();
    const divider = container.querySelector("[role=separator]")!;
    expect(divider.getAttribute("aria-valuenow")).toBe("500");
    await act(async () => observedResize([{ contentRect: { width: 600 } } as ResizeObserverEntry], {} as ResizeObserver));
    expect(divider.getAttribute("aria-valuenow")).toBe("308");
    await act(async () => observedResize([{ contentRect: { width: 1200 } } as ResizeObserverEntry], {} as ResizeObserver));
    expect(divider.getAttribute("aria-valuenow")).toBe("500");
  });
  it("ignores obsolete height preferences when restoring the new vertical chat pane", async () => {
    localStorage.setItem("hypervibes:chart-panel:fixture", JSON.stringify({ open: false, height: 650 }));
    await render();
    expect(container.querySelector<HTMLElement>("[aria-label='Chart chat panel']")!.hidden).toBe(true);
    await click("Show chat");
    expect(container.querySelector("[role=separator]")!.getAttribute("aria-valuenow")).toBe("360");
  });
  it("autosaves selection with revision checks and shares the root chat to the right of the chart", async () => {
    await render(); expect(mocks.create).not.toHaveBeenCalled();
    await select("Instrument", "ETH");
    expect(container.querySelector("button[aria-label=Instrument] img")!.getAttribute("src")).toBe("/currency/ETH.svg");
    expect(mocks.change).toHaveBeenLastCalledWith({ data: expect.objectContaining({ layoutId: layout.id, revision: 0, action: "edit", edit: { label: "Change instrument", settings: { coin: "ETH" } } }) });
    await select("Timeframe", "4h");
    expect(mocks.change).toHaveBeenLastCalledWith({ data: expect.objectContaining({ revision: 1 }) });
    expect(mocks.chatProps).toMatchObject({ initial: chat, embedded: true, visible: true, chartContext: { label: "My chart · ETH · 4h" } });
    const context = mocks.chatProps.chartContext as { capture: (image: boolean) => Promise<unknown> };
    await context.capture(false); expect(mocks.capture).toHaveBeenCalledWith(layout, "", false);
  });
  it("hides annotation tools initially and exits drawing mode when the toolbar is closed", async () => {
    await render();
    expect(container.querySelector("[role=toolbar][aria-label='Drawing tools']")).toBeNull();
    const toggle = container.querySelector("[aria-label='Show annotation tools']")!;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await click("Show annotation tools");
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelector("[role=toolbar][aria-label='Drawing tools']")).not.toBeNull();
    await click("Trend line");
    expect(mocks.chartProps.tool).toBe("trend");
    expect(mocks.chatProps.chartContext).toMatchObject({ disabled: true });
    await click("Hide annotation tools");
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(container.querySelector("[role=toolbar][aria-label='Drawing tools']")).toBeNull();
    expect(mocks.chartProps.tool).toBe("select");
    expect(mocks.chatProps.chartContext).toMatchObject({ disabled: false });
    expect(mocks.change).not.toHaveBeenCalled();
    await click("Show annotation tools");
    expect(container.querySelector("[aria-label=Select]")!.getAttribute("aria-pressed")).toBe("true");
  });
  it("reveals basic drawing tools on demand and saves selected objects without a discuss button", async () => {
    await render(); await click("Show annotation tools");
    for (const label of ["Horizontal line", "Horizontal ray", "Trend line", "Rectangle zone", "Text", "Callout", "Arrow"]) expect(container.querySelector(`[aria-label="${label}"]`)).not.toBeNull();
    await click("Place fixture level");
    expect(mocks.change).toHaveBeenCalledWith({ data: expect.objectContaining({ edit: expect.objectContaining({ upsert: [expect.objectContaining({ kind: "horizontal", coin: "BTC", author: "user" })] }) }) });
    expect(container.querySelector("[aria-label='Objects and notes']")).toBeNull();
    expect(container.querySelector("[aria-label='Chart canvas']")!.textContent).toContain("horizontal");
    expect(document.activeElement).toBe(container.querySelector("[aria-label='Chart canvas']"));
    expect(container.textContent).not.toContain("Discuss in chat");
    expect(mocks.chatProps).not.toHaveProperty("draftPrompt");
    const context = mocks.chatProps.chartContext as { capture: (image: boolean) => Promise<unknown> };
    await context.capture(false); expect(mocks.capture).toHaveBeenLastCalledWith(layout, layout.document.drawings[0]!.id, false);
  });
  it("explains timeframe-filtered drawings and lets the user show them on all timeframes", async () => {
    const d = { ...newDrawing("horizontal", "BTC", [{ time: 3600, price: 100 }]), name: "Agent support", author: "agent" as const, intervals: ["1h" as const] };
    layout = { ...layout, document: { ...layout.document, interval: "1d", drawings: [d] } }; initial = [layout];
    await render();
    expect(container.textContent).toContain("1 drawing hidden on 1d by timeframe settings");
    await click("Show on all timeframes");
    expect(mocks.change).toHaveBeenCalledWith({ data: expect.objectContaining({ edit: { label: "Show drawings on all timeframes", upsert: [{ ...d, intervals: [] }] } }) });
    expect(container.textContent).not.toContain("hidden on 1d by timeframe settings");
    expect(layout.document.drawings).toHaveLength(1);
  });
  it("collapses chat without losing its component and persists keyboard resizing", async () => {
    await render(); await click("Hide chat");
    expect(mocks.chatProps.visible).toBe(false);
    expect(container.querySelector("[aria-label='Root chat fixture']")).not.toBeNull();
    await click("Show chat");
    const handle = container.querySelector<HTMLElement>("[role=separator]")!;
    expect(handle.getAttribute("aria-orientation")).toBe("vertical");
    await act(async () => handle.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true })));
    expect(handle.getAttribute("aria-valuenow")).toBe("380");
    expect(JSON.parse(localStorage.getItem("hypervibes:chart-panel:fixture")!)).toEqual({ open: true, width: 380 });
    expect(container.querySelector<HTMLElement>("[aria-label='Chart chat panel']")!.style.width).toBe("392px");
  });
  it("creates an idempotent default layout for a fresh workspace", async () => {
    initial = []; await render();
    expect(mocks.create).toHaveBeenCalledTimes(1);
    expect(mocks.create).toHaveBeenCalledWith({ data: expect.objectContaining({ settings: { name: "My chart", coin: "BTC", interval: "1h" } }) });
    expect(container.textContent).toContain("Saved");
  });
  it("surfaces save conflicts and reconciles via SSE without silently reapplying the edit", async () => {
    mocks.change.mockRejectedValue(new Error("Chart changed elsewhere"));
    await render(); await select("Instrument", "ETH");
    expect(container.querySelector("[role=alert]")!.textContent).toContain("changed elsewhere");
    expect(mocks.change).toHaveBeenCalledTimes(1); expect(mocks.list).not.toHaveBeenCalled();
    expect(container.querySelector("button[aria-label=Instrument]")!.textContent).toContain("BTC");
    expect(document.querySelector("dialog [role=alert]")!.textContent).toContain("changed elsewhere");
    await act(async () => LayoutSource.instances.at(-1)!.emit([{ ...layout, revision: 1, document: { ...layout.document, coin: "ETH" } }]));
    expect(container.querySelector("button[aria-label=Instrument]")!.textContent).toContain("ETH");
    expect(mocks.change).toHaveBeenCalledTimes(1);
  });
});
