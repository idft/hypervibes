import { afterEach, describe, expect, it, vi } from "vitest";

type Series = { setData: (data: unknown) => void };

const addSeries = vi.fn<(definition: unknown, options?: unknown, pane?: number) => Series>(() => ({ setData: vi.fn() }));
const addPane = vi.fn(() => ({ paneIndex: () => 1 }));
const applyOptions = vi.fn();
const remove = vi.fn();
const createSeriesMarkers = vi.fn<(series: Series, markers: unknown[]) => void>();

vi.mock("lightweight-charts", () => ({
  CandlestickSeries: "candles",
  LineSeries: "line",
  createChart: vi.fn(() => ({ addSeries, addPane, applyOptions, remove })),
  createSeriesMarkers: (series: Series, markers: unknown[]) => createSeriesMarkers(series, markers),
}));

import { initIndicators, installIndicatorsLifecycle } from "./indicators";

afterEach(() => {
  document.body.replaceChildren();
  vi.unstubAllGlobals();
  addSeries.mockClear();
  addPane.mockClear();
  applyOptions.mockClear();
  remove.mockClear();
  createSeriesMarkers.mockClear();
});

describe("indicators", () => {
  it("imports Pine source into the editor", async () => {
    document.body.innerHTML = `
      <section data-agent-indicators>
        <textarea data-indicator-source></textarea>
        <input data-indicator-import type="file">
        <button data-indicator-import-trigger></button>
        <p data-indicator-import-error></p>
      </section>
    `;
    const file = document.querySelector<HTMLInputElement>("[data-indicator-import]");
    Object.defineProperty(file, "files", {
      value: [{ size: 100, text: vi.fn().mockResolvedValue("indicator(\"EMA\")") }],
    });

    initIndicators();
    file?.dispatchEvent(new Event("change"));
    await Promise.resolve();

    expect(document.querySelector<HTMLTextAreaElement>("[data-indicator-source]")?.value).toBe("indicator(\"EMA\")");
    expect(document.querySelector("[data-indicator-import-error]")?.textContent).toBe("");
  });

  it("does not fetch chart data without an indicator and instrument", () => {
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    document.body.innerHTML = `
      <section data-agent-indicators>
        <div data-indicator-chart data-agent-key="agent"></div>
      </section>
    `;

    initIndicators();

    expect(fetch).not.toHaveBeenCalled();
    expect(document.querySelector("[data-indicator-chart]")?.textContent).toContain("unavailable");
  });

  it("renders an empty state when no successful run is available", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false }));
    document.body.innerHTML = `
      <section data-agent-indicators>
        <div data-indicator-chart data-agent-key="agent" data-indicator-id="indicator-id" data-instrument-id="BTC" data-timeframe="1h"></div>
      </section>
    `;

    initIndicators();
    await Promise.resolve();

    expect(document.querySelector("[data-indicator-chart]")?.textContent).toContain("No successful BTC run");
  });

  it("loads each instrument chart independently", async () => {
    const fetch = vi.fn().mockResolvedValue({ ok: false });
    vi.stubGlobal("fetch", fetch);
    document.body.innerHTML = `
      <section data-agent-indicators>
        <div data-indicator-chart data-agent-key="agent" data-indicator-id="indicator-id" data-instrument-id="BTC" data-timeframe="1h"></div>
        <div data-indicator-chart data-agent-key="agent" data-indicator-id="indicator-id" data-instrument-id="ETH" data-timeframe="4h"></div>
      </section>
    `;

    initIndicators();
    await Promise.resolve();

    expect(fetch).toHaveBeenCalledTimes(2);
    expect(fetch).toHaveBeenCalledWith(expect.stringContaining("instrument_id=BTC"));
    expect(fetch).toHaveBeenCalledWith(expect.stringContaining("instrument_id=ETH"));
    expect(fetch).toHaveBeenCalledWith(expect.stringContaining("timeframe=4h"));
  });

  it("renders non-overlay plots in a dedicated pane and cleans up before an HTMX swap", async () => {
    let disconnected = false;
    class TestResizeObserver {
      disconnect() { disconnected = true; }
      observe() {}
    }
    vi.stubGlobal("ResizeObserver", TestResizeObserver);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({
        indicator: { overlay: false },
        candles: [{ time: 1, open: 10, high: 12, low: 9, close: 11 }],
        plots: [{ title: "RSI", values: [{ time: 1, value: 50 }] }],
        markers: [],
      }),
    }));
    document.body.innerHTML = `
      <section data-agent-indicators>
        <div data-indicator-chart data-agent-key="agent" data-indicator-id="indicator-id" data-instrument-id="BTC" data-timeframe="1h"></div>
      </section>
    `;

    initIndicators();
    await Promise.resolve();
    await Promise.resolve();

    expect(addPane).toHaveBeenCalledOnce();
    expect(addSeries).toHaveBeenCalledTimes(2);
    expect(addSeries.mock.calls[1]?.[2]).toBe(1);
    expect(createSeriesMarkers).not.toHaveBeenCalled();

    installIndicatorsLifecycle();
    document.dispatchEvent(new CustomEvent("htmx:beforeSwap", { detail: { target: document.body } }));

    expect(disconnected).toBe(true);
    expect(remove).toHaveBeenCalledOnce();
  });

  it("maps Pine marker output onto the candlestick series", async () => {
    class TestResizeObserver {
      disconnect() {}
      observe() {}
    }
    vi.stubGlobal("ResizeObserver", TestResizeObserver);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({
        indicator: { overlay: true },
        candles: [{ time: 1, open: 10, high: 12, low: 9, close: 11 }],
        plots: [{ title: "EMA", values: [{ time: 1, value: 11 }] }],
        markers: [
          { kind: "plotshape", time: 1, title: "Buy", text: "BUY", style: "triangleup", location: "belowbar", color: { red: 10, green: 20, blue: 30, transparency: 25 }, text_color: null, size: "huge", price: null },
          { kind: "plotshape", time: 1, title: "Sell", text: "SELL", style: "triangledown", location: "abovebar", color: null, text_color: null, size: "tiny", price: null },
          { kind: "plotshape", time: 1, title: "Square", text: "123", style: "diamond", location: "absolute", color: null, text_color: { red: 1, green: 2, blue: 3, transparency: 150 }, size: "small", price: 0 },
          { kind: "plotshape", time: 1, title: "Circle", text: "", style: "xcross", location: "top", color: null, text_color: null, size: "normal", price: null },
          { kind: "plotchar", time: 1, title: "One", character: "1", text: "Stage", location: "belowbar", color: null, text_color: null, size: "large", price: null },
          { kind: "plotchar", time: 1, title: "Two", character: "2", text: "", location: "bottom", color: null, text_color: null, size: "auto", price: null },
          { kind: "plotchar", time: 1, title: "Three", character: "3", text: "", location: "absolute", color: null, text_color: null, size: "normal", price: 0 },
          { kind: "plotarrow", time: 1, title: "Up", direction: "up", value: 2.5, color: null },
          { kind: "plotarrow", time: 1, title: "Down", direction: "down", value: -3, color: { red: 255, green: 0, blue: 0, transparency: 0 } },
        ],
      }),
    }));
    document.body.innerHTML = `
      <section data-agent-indicators>
        <div data-indicator-chart data-agent-key="agent" data-indicator-id="indicator-id" data-instrument-id="BTC" data-timeframe="1h"></div>
      </section>
    `;

    initIndicators();
    await Promise.resolve();
    await Promise.resolve();

    const candleSeries = addSeries.mock.results[0]?.value;
    const lineSeries = addSeries.mock.results[1]?.value;
    expect(createSeriesMarkers).toHaveBeenCalledOnce();
    expect(createSeriesMarkers.mock.calls[0]?.[0]).toBe(candleSeries);
    expect(createSeriesMarkers.mock.calls[0]?.[0]).not.toBe(lineSeries);
    expect(createSeriesMarkers.mock.calls[0]?.[1]).toEqual([
      { time: 1, position: "belowBar", shape: "arrowUp", color: "rgba(10, 20, 30, 0.75)", size: 2, text: "BUY" },
      { time: 1, position: "aboveBar", shape: "arrowDown", color: "#a1a1aa", size: 0.5, text: "SELL" },
      { time: 1, position: "atPriceMiddle", price: 0, shape: "square", color: "rgba(1, 2, 3, 0)", size: 0.75, text: "123" },
      { time: 1, position: "aboveBar", shape: "circle", color: "#a1a1aa", size: 1 },
      { time: 1, position: "belowBar", shape: "circle", color: "#a1a1aa", size: 1.5, text: "1 Stage" },
      { time: 1, position: "belowBar", shape: "circle", color: "#a1a1aa", size: 1, text: "2" },
      { time: 1, position: "atPriceMiddle", price: 0, shape: "circle", color: "#a1a1aa", size: 1, text: "3" },
      { time: 1, position: "belowBar", shape: "arrowUp", color: "#a1a1aa", size: 1 },
      { time: 1, position: "aboveBar", shape: "arrowDown", color: "rgba(255, 0, 0, 1)", size: 1 },
    ]);
  });

  it("bounds timeframe rows and switches result panels", () => {
    document.body.innerHTML = `
      <section data-agent-indicators>
        <div data-indicator-timeframe-list><div data-indicator-timeframe-row><input name="timeframe"><button data-indicator-timeframe-remove></button></div></div>
        <button data-indicator-timeframe-add></button>
        <select data-indicator-timeframe-selector><option value="15m">15m</option><option value="1h">1h</option></select>
        <div data-indicator-timeframe-panel="15m"></div>
        <div data-indicator-timeframe-panel="1h" hidden></div>
      </section>
    `;
    initIndicators();
    const add = document.querySelector<HTMLButtonElement>("[data-indicator-timeframe-add]");
    add?.click();
    expect(document.querySelectorAll("[data-indicator-timeframe-row]")).toHaveLength(2);

    const selector = document.querySelector<HTMLSelectElement>("[data-indicator-timeframe-selector]");
    if (selector) selector.value = "1h";
    selector?.dispatchEvent(new Event("change"));
    expect(document.querySelector<HTMLElement>('[data-indicator-timeframe-panel="15m"]')?.hidden).toBe(true);
    expect(document.querySelector<HTMLElement>('[data-indicator-timeframe-panel="1h"]')?.hidden).toBe(false);
  });
});
