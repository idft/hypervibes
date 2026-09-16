import { CandlestickSeries, LineSeries, createChart, type UTCTimestamp } from "lightweight-charts";

type ChartHandle = { remove: () => void };
type ChartData = {
  indicator: { overlay: boolean };
  candles: Array<{ time: UTCTimestamp; open: number; high: number; low: number; close: number }>;
  plots: Array<{ title: string; values: Array<{ time: UTCTimestamp; value: number }> }>;
};

const MAX_IMPORT_BYTES = 256 * 1024;
const charts = new WeakMap<HTMLElement, ChartHandle>();

function removeChart(element: HTMLElement) {
  charts.get(element)?.remove();
  charts.delete(element);
}

function chartOptions(element: HTMLElement) {
  return {
    width: element.clientWidth,
    height: 360,
    layout: { background: { color: "#09090b" }, textColor: "#a1a1aa" },
    grid: { vertLines: { color: "#27272a" }, horzLines: { color: "#27272a" } },
  };
}

export function initIndicators(root: ParentNode = document) {
  root.querySelectorAll<HTMLElement>("[data-agent-indicators]").forEach((page) => {
    if (page.dataset.indicatorsBound === "true") return;
    page.dataset.indicatorsBound = "true";
    const source = page.querySelector<HTMLTextAreaElement>("[data-indicator-source]");
    const file = page.querySelector<HTMLInputElement>("[data-indicator-import]");
    const importError = page.querySelector<HTMLElement>("[data-indicator-import-error]");
    page.querySelector<HTMLButtonElement>("[data-indicator-import-trigger]")?.addEventListener("click", () => file?.click());
    file?.addEventListener("change", () => {
      const selected = file.files?.[0];
      file.value = "";
      if (!selected || !source) return;
      if (selected.size > MAX_IMPORT_BYTES) {
        if (importError) importError.textContent = "The source file is too large to import.";
        return;
      }
      void selected.text().then((text) => {
        source.value = text;
        if (importError) importError.textContent = "";
      }).catch(() => {
        if (importError) importError.textContent = "The source file could not be read.";
      });
    });

    const chartElement = page.querySelector<HTMLElement>("[data-indicator-chart]");
    const definition = page.querySelector<HTMLSelectElement>("[data-indicator-chart-definition]");
    const instrument = page.querySelector<HTMLSelectElement>("[data-indicator-chart-instrument]");
    if (!chartElement || !definition || !instrument) return;
    const load = async () => {
      removeChart(chartElement);
      chartElement.replaceChildren();
      const agentKey = chartElement.dataset.agentKey;
      if (!agentKey || !definition.value || !instrument.value) {
        chartElement.textContent = "Select an indicator and instrument to load chart data.";
        return;
      }
      try {
        const response = await fetch(`/agents/${encodeURIComponent(agentKey)}/indicators/chart-data?indicator_id=${encodeURIComponent(definition.value)}&instrument_id=${encodeURIComponent(instrument.value)}`);
        if (!response.ok) {
          chartElement.textContent = "No successful run is available for this selection.";
          return;
        }
        const data = await response.json() as ChartData;
        if (data.candles.length === 0) {
          chartElement.textContent = "No candle data is available.";
          return;
        }
        const chart = createChart(chartElement, chartOptions(chartElement));
        chart.addSeries(CandlestickSeries).setData(data.candles);
        const plotPane = data.indicator.overlay ? 0 : chart.addPane().paneIndex();
        data.plots.forEach((plot) => chart.addSeries(LineSeries, { title: plot.title }, plotPane).setData(plot.values));
        const observer = new ResizeObserver(() => chart.applyOptions({ width: chartElement.clientWidth }));
        observer.observe(chartElement);
        charts.set(chartElement, { remove: () => { observer.disconnect(); chart.remove(); } });
      } catch {
        chartElement.textContent = "Chart data could not be loaded.";
      }
    };
    definition.addEventListener("change", () => { void load(); });
    instrument.addEventListener("change", () => { void load(); });
    void load();
  });
}

export function installIndicatorsLifecycle() {
  document.addEventListener("htmx:beforeSwap", (event) => {
    const target = (event as CustomEvent<{ target?: unknown }>).detail.target;
    if (target instanceof Element) target.querySelectorAll<HTMLElement>("[data-indicator-chart]").forEach(removeChart);
  });
}
