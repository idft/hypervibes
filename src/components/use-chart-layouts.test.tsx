// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChartLayout } from "../charts/contracts.js";
import { useChartLayouts } from "./use-chart-layouts.js";

const mocks = vi.hoisted(() => ({ create: vi.fn(), change: vi.fn(), remove: vi.fn(), list: vi.fn() }));
vi.mock("../web/charts.js", () => ({ createChartLayout: mocks.create, changeChartLayout: mocks.change, deleteChartLayout: mocks.remove, getChartLayouts: mocks.list }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
class LayoutSource extends EventTarget {
  static instances: LayoutSource[] = [];
  close = vi.fn();
  onerror: (() => void) | null = null;
  constructor(readonly url: string) { super(); LayoutSource.instances.push(this); }
  emit(event: string, data: unknown) { this.dispatchEvent(new MessageEvent(event, { data: JSON.stringify(data) })); }
}
const fixture = (id = "first", revision = 0): ChartLayout => ({ id, revision, document: { name: id, coin: "BTC", interval: "1h", drawings: [], notes: "", hideDrawings: false, magnet: true }, updatedAt: revision, canUndo: revision > 0, canRedo: false, lastChange: "Saved" });

describe("SSE chart layouts", () => {
  let root: Root | undefined, container: HTMLDivElement, state: ReturnType<typeof useChartLayouts>, initial: ChartLayout[];
  function Fixture() { state = useChartLayouts("agent", initial, ["BTC"]); return null; }
  const source = () => LayoutSource.instances.at(-1)!;
  async function render() { await act(async () => root!.render(<Fixture />)); }
  async function push(layouts: ChartLayout[]) { await act(async () => source().emit("chart-layouts", layouts)); }
  beforeEach(() => {
    vi.resetAllMocks(); localStorage.clear(); LayoutSource.instances = []; vi.stubGlobal("EventSource", LayoutSource);
    initial = [fixture()]; container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => {
    if (root) await act(async () => root!.unmount());
    container.remove(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  });

  it("uses one persistent subscription, makes no polling requests, and applies reconnect snapshots", async () => {
    vi.useFakeTimers(); const interval = vi.spyOn(window, "setInterval");
    await render(); await push(initial);
    expect(source().url).toBe("/api/agents/agent/chart/layouts/events");
    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
    expect(interval).not.toHaveBeenCalled(); expect(mocks.list).not.toHaveBeenCalled();
    await render(); expect(LayoutSource.instances).toHaveLength(1);
    const remote = fixture("other-tab", 5); await push([remote]);
    expect(state!.layouts).toEqual([remote]); expect(state!.layout).toEqual(remote);
    await act(async () => source().onerror?.());
    expect(state!.error).toContain("Reconnecting");
    const latest = { ...remote, revision: 6, document: { ...remote.document, notes: "Edited while offline" } };
    await push([latest]);
    expect(state!.layout).toEqual(latest); expect(state!.error).toBe("");
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it("keeps a newer streamed revision when an older mutation response arrives", async () => {
    let finish!: (value: unknown) => void;
    mocks.change.mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    await render(); await push(initial);
    await act(async () => { void state!.change("edit", { label: "My edit", settings: { notes: "Mine" } }); });
    expect(state!.pending).toBe(true);
    const latest = fixture("first", 2); await push([latest]);
    await act(async () => finish({ ok: true, data: fixture("first", 1) }));
    expect(state!.layout).toEqual(latest); expect(state!.pending).toBe(false);
  });

  it("accepts a newer committed response despite an older SSE update and allows the next edit immediately", async () => {
    let finish!: (value: unknown) => void;
    mocks.change.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    await render(); await push(initial);
    await act(async () => { void state!.change("edit", { label: "First move" }); });
    await push([fixture("first", 0)]);
    await act(async () => finish({ ok: true, data: fixture("first", 1) }));
    expect(state!.layout?.revision).toBe(1); expect(state!.pending).toBe(false);
    mocks.change.mockResolvedValueOnce({ ok: true, data: fixture("first", 2) });
    await act(async () => { await state!.change("edit", { label: "Second move" }); });
    expect(mocks.change).toHaveBeenLastCalledWith({ data: expect.objectContaining({ revision: 1 }) });
    expect(state!.layout?.revision).toBe(2);
    expect(mocks.list).not.toHaveBeenCalled();
  });

  it("does not downgrade a committed layout when an older SSE frame arrives late", async () => {
    mocks.change.mockResolvedValueOnce({ ok: true, data: fixture("first", 1) });
    await render(); await push(initial);
    await act(async () => { await state!.change("edit", { label: "Move" }); });
    await push([fixture("first", 0), fixture("other", 2)]);
    expect(state!.layouts).toEqual([fixture("first", 1), fixture("other", 2)]);
    await push([fixture("first", 2), fixture("other", 3)]);
    expect(state!.layout?.revision).toBe(2);
    await push([fixture("other", 3)]);
    expect(state!.layouts).toEqual([fixture("other", 3)]);
  });

  it("does not resurrect a remotely deleted layout when a mutation returns late", async () => {
    let finish!: (value: unknown) => void;
    mocks.change.mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    await render(); await push(initial);
    await act(async () => { void state!.change("edit", { label: "My edit" }); });
    await push([]);
    await act(async () => finish({ ok: true, data: fixture("first", 1) }));
    expect(state!.layouts).toEqual([]); expect(mocks.create).not.toHaveBeenCalled();
  });

  it("does not restore a newly created layout deleted before its response arrives", async () => {
    let finish!: (value: unknown) => void;
    mocks.create.mockReturnValue(new Promise((resolve) => { finish = resolve; }));
    await render(); await push(initial);
    await act(async () => { void state!.create("New", false, "new"); });
    await push([fixture("new"), ...initial]); await push(initial);
    await act(async () => finish({ ok: true, data: fixture("new") }));
    expect(state!.layouts).toEqual(initial); expect(state!.layout).toEqual(initial[0]);
  });

  it("waits for the initial authoritative snapshot before creating a default layout", async () => {
    initial = []; await render();
    expect(state!.ready).toBe(false); expect(mocks.create).not.toHaveBeenCalled();
    await push([fixture("created-in-another-tab")]);
    expect(state!.ready).toBe(true); expect(mocks.create).not.toHaveBeenCalled();
  });

  it("initializes an empty workspace once and remembers the chosen layout", async () => {
    mocks.create.mockResolvedValue({ ok: true, data: fixture("default") });
    initial = []; await render(); await push([]);
    expect(mocks.create).toHaveBeenCalledTimes(1);
    expect(state!.layout?.id).toBe("default");
    expect(localStorage.getItem("hypervibes:chart-layout:agent")).toBe("default");
    await push([fixture("default")]); await push([]);
    expect(mocks.create).toHaveBeenCalledTimes(1);
  });

  it("preserves save errors while reconciling conflicts through SSE, without a GET or retry", async () => {
    mocks.change.mockResolvedValue({ ok: false, error: "Chart changed elsewhere" });
    await render(); await push(initial);
    await act(async () => { await state!.change("edit", { label: "My edit" }); });
    const latest = fixture("first", 3); await push([latest]);
    expect(state!.layout).toEqual(latest); expect(state!.error).toBe("Chart changed elsewhere");
    expect(mocks.list).not.toHaveBeenCalled(); expect(mocks.change).toHaveBeenCalledTimes(1);
  });

  it("closes on navigation and expired sessions, and reports terminal stream errors", async () => {
    await render(); await push(initial);
    await act(async () => source().emit("unavailable", { message: "Connection interrupted" }));
    expect(state!.error).toBe("Connection interrupted");
    await push(initial); expect(state!.error).toBe("");
    const location = window.location;
    vi.spyOn(window, "location", "get").mockReturnValue({ ...location, assign: vi.fn() } as unknown as Location);
    await act(async () => source().emit("unauthorized", {}));
    expect(source().close).toHaveBeenCalled(); expect(window.location.assign).toHaveBeenCalledWith("/login");
    await act(async () => source().emit("stream-error", { message: "Reload the page" }));
    expect(state!.error).toBe("Reload the page");
    const closed = source();
    await act(async () => root!.unmount()); root = undefined;
    expect(closed.close).toHaveBeenCalled();
    closed.emit("chart-layouts", [fixture("ignored-after-unmount")]);
    expect(state!.layouts).toEqual(initial);
  });
});
