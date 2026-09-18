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

    page.querySelectorAll<HTMLElement>("[data-indicator-chart]").forEach((chartElement) => {
      const load = async () => {
      removeChart(chartElement);
      chartElement.replaceChildren();
      const agentKey = chartElement.dataset.agentKey;
      const indicatorId = chartElement.dataset.indicatorId;
      const instrumentId = chartElement.dataset.instrumentId;
      if (!agentKey || !indicatorId || !instrumentId) {
        chartElement.textContent = "Chart data is unavailable.";
        return;
      }
      try {
        const response = await fetch(`/agents/${encodeURIComponent(agentKey)}/indicators/chart-data?indicator_id=${encodeURIComponent(indicatorId)}&instrument_id=${encodeURIComponent(instrumentId)}`);
        if (!response.ok) {
          chartElement.textContent = `No successful ${instrumentId} run is available.`;
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
      void load();
    });
  });
}

export function installIndicatorsLifecycle() {
  document.addEventListener("htmx:beforeSwap", (event) => {
    const target = (event as CustomEvent<{ target?: unknown }>).detail.target;
    if (target instanceof Element) target.querySelectorAll<HTMLElement>("[data-indicator-chart]").forEach(removeChart);
  });
}
