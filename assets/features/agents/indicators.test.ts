import { afterEach, describe, expect, it, vi } from "vitest";

type Series = { setData: (data: unknown) => void };

const addSeries = vi.fn<(definition: unknown, options?: unknown, pane?: number) => Series>(() => ({ setData: vi.fn() }));
const addPane = vi.fn(() => ({ paneIndex: () => 1 }));
const applyOptions = vi.fn();
const remove = vi.fn();

vi.mock("lightweight-charts", () => ({
  CandlestickSeries: "candles",
  LineSeries: "line",
  createChart: vi.fn(() => ({ addSeries, addPane, applyOptions, remove })),
}));

import { initIndicators, installIndicatorsLifecycle } from "./indicators";

afterEach(() => {
  document.body.replaceChildren();
  vi.unstubAllGlobals();
  addSeries.mockClear();
  addPane.mockClear();
  applyOptions.mockClear();
  remove.mockClear();
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
        <div data-indicator-chart data-agent-key="agent" data-indicator-id="indicator-id" data-instrument-id="BTC"></div>
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
        <div data-indicator-chart data-agent-key="agent" data-indicator-id="indicator-id" data-instrument-id="BTC"></div>
        <div data-indicator-chart data-agent-key="agent" data-indicator-id="indicator-id" data-instrument-id="ETH"></div>
      </section>
    `;

    initIndicators();
    await Promise.resolve();

    expect(fetch).toHaveBeenCalledTimes(2);
    expect(fetch).toHaveBeenCalledWith(expect.stringContaining("instrument_id=BTC"));
    expect(fetch).toHaveBeenCalledWith(expect.stringContaining("instrument_id=ETH"));
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
      }),
    }));
    document.body.innerHTML = `
      <section data-agent-indicators>
        <div data-indicator-chart data-agent-key="agent" data-indicator-id="indicator-id" data-instrument-id="BTC"></div>
      </section>
    `;

    initIndicators();
    await Promise.resolve();
    await Promise.resolve();

    expect(addPane).toHaveBeenCalledOnce();
    expect(addSeries).toHaveBeenCalledTimes(2);
    expect(addSeries.mock.calls[1]?.[2]).toBe(1);

    installIndicatorsLifecycle();
    document.dispatchEvent(new CustomEvent("htmx:beforeSwap", { detail: { target: document.body } }));

    expect(disconnected).toBe(true);
    expect(remove).toHaveBeenCalledOnce();
  });
});
