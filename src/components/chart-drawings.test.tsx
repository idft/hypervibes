// @vitest-environment jsdom
import { act, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChartDrawings, type ChartProjection } from "./chart-drawings.js";
import { newDrawing, type ChartDocument, type ChartDrawing, type DrawingKind } from "../charts/contracts.js";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
function pointerEvent(type: string, x: number, y: number, ctrlKey = false, pointerId = 1) {
  return Object.assign(new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0, ctrlKey }), { pointerId });
}
const projection: ChartProjection = { width: 500, height: 400, x: (time) => time / 10, y: (price) => 400 - price, point: (x, y, snap) => ({ time: snap ? Math.round(x / 10) * 100 : x * 10, price: snap ? Math.round((400 - y) / 10) * 10 : 400 - y }) };
function Fixture(props: { document: ChartDocument; revision: number; disabled: boolean; tool: DrawingKind | "select"; selected: string; onSave: (drawing: ChartDrawing) => Promise<number | undefined>; onSelect: (id: string) => void; onEdit: () => void; onDrawn: () => void }) {
  const svg = useRef<SVGSVGElement>(null);
  return <ChartDrawings {...props} svgRef={svg} projection={projection} />;
}
describe("chart drawing interactions", () => {
  let root: Root, container: HTMLDivElement, documentValue: ChartDocument;
  const save = vi.fn(), select = vi.fn(), edit = vi.fn(), drawn = vi.fn();
  beforeEach(() => {
    vi.resetAllMocks();
    documentValue = { name: "Test", coin: "BTC", interval: "1h", drawings: [], notes: "", magnet: true, hideDrawings: false };
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    const captures = new WeakMap<SVGElement, Set<number>>();
    SVGElement.prototype.setPointerCapture = vi.fn(function (this: SVGElement, id: number) { const ids = captures.get(this) ?? new Set<number>(); ids.add(id); captures.set(this, ids); });
    SVGElement.prototype.hasPointerCapture = vi.fn(function (this: SVGElement, id: number) { return captures.get(this)?.has(id) ?? false; });
    SVGElement.prototype.releasePointerCapture = vi.fn(function (this: SVGElement, id: number) { captures.get(this)?.delete(id); this.dispatchEvent(pointerEvent("lostpointercapture", 0, 0, false, id)); });
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  async function render(tool: DrawingKind | "select", selected = "", revision = 0, disabled = false) { await act(async () => root.render(<Fixture key={tool} document={documentValue} revision={revision} disabled={disabled} tool={tool} selected={selected} onSave={save} onSelect={select} onEdit={edit} onDrawn={drawn} />)); }
  async function pointer(type: string, target: Element, x: number, y: number, ctrlKey = false) { await act(async () => target.dispatchEvent(pointerEvent(type, x, y, ctrlKey))); }
  const svg = () => container.querySelector("svg")!;
  it("creates snapped one-anchor levels and two-anchor zones, with a temporary magnet override", async () => {
    await render("horizontal"); await pointer("pointerdown", svg(), 23, 297);
    expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "horizontal", points: [{ time: 200, price: 100 }] }));
    expect(drawn).toHaveBeenCalledOnce();
    await render("rectangle"); await pointer("pointerdown", svg(), 23, 297, true);
    expect(save).toHaveBeenCalledTimes(1);
    await pointer("pointermove", svg(), 50, 250, true);
    expect(container.querySelector("rect")).not.toBeNull();
    await pointer("pointerdown", svg(), 50, 250, true);
    expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "rectangle", points: [{ time: 230, price: 103 }, { time: 500, price: 150 }] }));
  });
  it("selects/edits objects, drags their handles and leaves locked objects unchanged", async () => {
    const d = newDrawing("trend", "BTC", [{ time: 100, price: 100 }, { time: 200, price: 150 }]); documentValue.drawings = [d];
    await render("select", d.id);
    await pointer("pointerdown", container.querySelector("circle")!, 10, 300);
    await pointer("pointermove", svg(), 30, 280);
    await pointer("pointerup", svg(), 30, 280);
    expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ id: d.id, points: [{ time: 300, price: 120 }, { time: 200, price: 150 }] }));
    await act(async () => container.querySelector("g")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(edit).toHaveBeenCalledOnce();
    documentValue = { ...documentValue, drawings: [{ ...d, locked: true }] };
    await render("select", d.id); save.mockClear();
    await pointer("pointerdown", container.querySelector("circle")!, 10, 300); await pointer("pointermove", svg(), 30, 280); await pointer("pointerup", svg(), 30, 280);
    expect(save).not.toHaveBeenCalled(); expect(select).toHaveBeenCalledWith(d.id);
  });
  it.each(["body", "handle"] as const)("keeps a dropped %s at its preview position through HTTP completion and SSE acknowledgement", async (target) => {
    let complete!: (revision: number | undefined) => void;
    save.mockReturnValue(new Promise<number | undefined>((resolve) => { complete = resolve; }));
    const original = newDrawing("trend", "BTC", [{ time: 100, price: 100 }, { time: 200, price: 150 }]);
    documentValue = { ...documentValue, magnet: false, drawings: [original] };
    await render("select", original.id);
    const path = () => container.querySelector("g path")!.getAttribute("d");
    const targetElement = target === "handle" ? container.querySelector("circle")! : container.querySelector("path[stroke='transparent']")!;
    await pointer("pointerdown", targetElement, 10, 300);
    await pointer("pointermove", svg(), 30, 280);
    const droppedPath = path();
    expect(droppedPath).not.toBe("M 10 300 L 20 250");
    await pointer("pointerup", svg(), 30, 280);
    expect(path()).toBe(droppedPath);
    await render("select", original.id, 0, true);
    expect(path()).toBe(droppedPath);
    await act(async () => complete(1));
    // HTTP can finish before the committed layout arrives over SSE.
    await render("select", original.id);
    expect(path()).toBe(droppedPath);
    expect(container.querySelector<SVGPathElement>("path[stroke='transparent']")!.style.cursor).toBe("progress");
    documentValue = { ...documentValue, drawings: [save.mock.calls[0]![0] as ChartDrawing] };
    await render("select", original.id, 1);
    expect(path()).toBe(droppedPath);
    expect(container.querySelector<SVGPathElement>("path[stroke='transparent']")!.style.cursor).toBe("move");
    // A later undo is authoritative, not overridden by a retained preview.
    documentValue = { ...documentValue, drawings: [original] };
    await render("select", original.id, 2);
    expect(path()).toBe("M 10 300 L 20 250");
  });
  it("hands off to the newest authoritative position when SSE arrives before the save response", async () => {
    let complete!: (revision: number) => void;
    save.mockReturnValue(new Promise<number>((resolve) => { complete = resolve; }));
    const original = newDrawing("trend", "BTC", [{ time: 100, price: 100 }, { time: 200, price: 150 }]);
    documentValue = { ...documentValue, magnet: false, drawings: [original] };
    await render("select", original.id);
    await pointer("pointerdown", container.querySelector("circle")!, 10, 300);
    await pointer("pointermove", svg(), 30, 280);
    await pointer("pointerup", svg(), 30, 280);
    const moved = save.mock.calls[0]![0] as ChartDrawing;
    documentValue = { ...documentValue, drawings: [moved] };
    await render("select", original.id, 1, true);
    expect(container.querySelector("g path")!.getAttribute("d")).toBe("M 30 280 L 20 250");
    documentValue = { ...documentValue, drawings: [{ ...moved, points: [{ time: 500, price: 200 }, { time: 600, price: 250 }] }] };
    await render("select", original.id, 2, true);
    await act(async () => complete(1));
    expect(container.querySelector("g path")!.getAttribute("d")).toBe("M 50 200 L 60 150");
  });
  it.each(["failed", "rejected"] as const)("rolls back a %s save rather than leaving an unsaved move on screen", async (outcome) => {
    let complete!: (revision: undefined) => void, reject!: (error: Error) => void;
    save.mockReturnValue(new Promise<undefined>((resolve, fail) => { complete = resolve; reject = fail; }));
    const original = newDrawing("horizontal", "BTC", [{ time: 100, price: 100 }]);
    documentValue = { ...documentValue, magnet: false, drawings: [original] };
    await render("select", original.id);
    await pointer("pointerdown", container.querySelector("circle")!, 10, 300);
    await pointer("pointermove", svg(), 30, 280);
    await pointer("pointerup", svg(), 30, 280);
    expect(container.querySelector("g path")!.getAttribute("d")).toBe("M 0 280 L 500 280");
    await act(async () => { if (outcome === "failed") complete(undefined); else reject(new Error("Save failed")); });
    expect(container.querySelector("g path")!.getAttribute("d")).toBe("M 0 300 L 500 300");
  });
  it("saves the last pointer position even when move and release share a React batch", async () => {
    save.mockResolvedValue(1);
    const original = newDrawing("trend", "BTC", [{ time: 100, price: 100 }, { time: 200, price: 150 }]);
    documentValue = { ...documentValue, magnet: false, drawings: [original] };
    await render("select", original.id);
    await pointer("pointerdown", container.querySelector("circle")!, 10, 300);
    await act(async () => {
      svg().dispatchEvent(pointerEvent("pointermove", 30, 280));
      svg().dispatchEvent(pointerEvent("pointerup", 30, 280));
    });
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ points: [{ time: 300, price: 120 }, { time: 200, price: 150 }] }));
    expect(container.querySelector("g path")!.getAttribute("d")).toBe("M 30 280 L 20 250");
  });
  it.each(["body", "handle"] as const)("allows repeated %s drags without reselection and keeps hit targets stable through each save", async (target) => {
    let complete!: (revision: number) => void;
    save.mockImplementation(() => new Promise<number>((resolve) => { complete = resolve; }));
    const original = newDrawing("trend", "BTC", [{ time: 100, price: 100 }, { time: 200, price: 150 }]);
    documentValue = { ...documentValue, magnet: false, drawings: [original] };
    await render("select", original.id);
    const hit = () => target === "body" ? container.querySelector("path[stroke='transparent']")! : container.querySelector("circle")!;
    const initialTarget = hit();
    for (let revision = 1; revision <= 3; revision++) {
      const before = documentValue.drawings[0]!.points[0]!;
      const x = before.time / 10, y = 400 - before.price;
      expect(hit()).toBe(initialTarget);
      await pointer("pointerdown", hit(), x, y);
      expect(svg().hasPointerCapture(1)).toBe(true);
      await pointer("pointermove", svg(), x + 20, y - 20);
      await pointer("pointerup", svg(), x + 20, y - 20);
      expect(svg().hasPointerCapture(1)).toBe(false);
      expect(save).toHaveBeenCalledTimes(revision);
      const moved = save.mock.calls.at(-1)![0] as ChartDrawing;
      expect(moved.points[0]).toEqual({ time: before.time + 200, price: before.price + 20 });
      await render("select", original.id, revision - 1, true);
      expect(hit()).toBe(initialTarget);
      // A blocked attempt must neither start another drag nor deselect the drawing.
      select.mockClear();
      await pointer("pointerdown", hit(), x + 20, y - 20);
      expect(svg().hasPointerCapture(1)).toBe(false); expect(select).not.toHaveBeenCalled();
      if (revision % 2 === 0) {
        documentValue = { ...documentValue, drawings: [moved] };
        await render("select", original.id, revision, true);
        await act(async () => complete(revision));
      } else {
        await act(async () => complete(revision));
        await render("select", original.id, revision - 1);
        expect(hit()).toBe(initialTarget);
        documentValue = { ...documentValue, drawings: [moved] };
      }
      await render("select", original.id, revision);
      expect(hit()).toBe(initialTarget);
      expect(container.querySelector("g path")!.getAttribute("d")).toContain(`M ${x + 20} ${y - 20}`);
    }
  });
  it("cancels a lost capture and permits another drag of the same selected annotation", async () => {
    const original = newDrawing("trend", "BTC", [{ time: 100, price: 100 }, { time: 200, price: 150 }]);
    documentValue = { ...documentValue, magnet: false, drawings: [original] };
    await render("select", original.id);
    await pointer("pointerdown", container.querySelector("circle")!, 10, 300);
    await pointer("pointermove", svg(), 30, 280);
    await act(async () => svg().releasePointerCapture(1));
    expect(container.querySelector("g path")!.getAttribute("d")).toBe("M 10 300 L 20 250");
    await pointer("pointerup", svg(), 30, 280);
    expect(save).not.toHaveBeenCalled();
    await pointer("pointerdown", container.querySelector("circle")!, 10, 300);
    await pointer("pointermove", svg(), 40, 270);
    await pointer("pointerup", svg(), 40, 270);
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ points: [{ time: 400, price: 130 }, { time: 200, price: 150 }] }));
  });
  it("ignores movement and release from other pointers during a drag", async () => {
    const original = newDrawing("trend", "BTC", [{ time: 100, price: 100 }, { time: 200, price: 150 }]);
    documentValue = { ...documentValue, magnet: false, drawings: [original] };
    await render("select", original.id);
    await pointer("pointerdown", container.querySelector("circle")!, 10, 300);
    await act(async () => {
      svg().dispatchEvent(pointerEvent("pointermove", 50, 250, false, 2));
      svg().dispatchEvent(pointerEvent("pointerup", 50, 250, false, 2));
    });
    expect(save).not.toHaveBeenCalled(); expect(svg().hasPointerCapture(1)).toBe(true);
    await pointer("pointermove", svg(), 30, 280);
    await pointer("pointerup", svg(), 30, 280);
    expect(save).toHaveBeenCalledOnce();
  });
  it("cancels a drag without saving or retaining its preview", async () => {
    const original = newDrawing("horizontal", "BTC", [{ time: 100, price: 100 }]);
    documentValue.drawings = [original]; await render("select", original.id);
    await pointer("pointerdown", container.querySelector("circle")!, 10, 300);
    await pointer("pointermove", svg(), 30, 280);
    await pointer("pointercancel", svg(), 30, 280);
    await pointer("pointerup", svg(), 30, 280);
    expect(save).not.toHaveBeenCalled();
    expect(container.querySelector("g path")!.getAttribute("d")).toBe("M 0 300 L 500 300");
  });
  it("renders only the current instrument/timeframe and safely escapes annotation text", async () => {
    const d = newDrawing("text", "BTC", [{ time: 100, price: 100 }]);
    documentValue.drawings = [{ ...d, text: "<script>not executable</script>" }, { ...d, id: crypto.randomUUID(), coin: "ETH" }, { ...d, id: crypto.randomUUID(), intervals: ["4h"] }];
    await render("select");
    expect(container.querySelectorAll("g")).toHaveLength(1); expect(container.querySelector("script")).toBeNull(); expect(container.textContent).toContain("<script>not executable</script>");
    documentValue = { ...documentValue, hideDrawings: true }; await render("select"); expect(container.querySelectorAll("g")).toHaveLength(0);
  });
  it("cancels an unfinished drawing when changing tools without saving an object", async () => {
    await render("trend"); await pointer("pointerdown", svg(), 10, 300); await pointer("pointermove", svg(), 30, 280);
    expect(container.querySelector("path")).not.toBeNull();
    await render("select"); expect(container.querySelector("path")).toBeNull(); expect(save).not.toHaveBeenCalled();
  });
});
