import { CandlestickSeries, LineSeries, createChart, type UTCTimestamp } from "lightweight-charts";

type ChartHandle = { remove: () => void };

const charts = new WeakMap<HTMLElement, ChartHandle>();

function removeChart(element: HTMLElement) {
  charts.get(element)?.remove();
  charts.delete(element);
}

export function initIndicators(root: ParentNode = document) {
  root.querySelectorAll<HTMLElement>("[data-agent-indicators]").forEach((page) => {
    if (page.dataset.indicatorsBound === "true") return;
    page.dataset.indicatorsBound = "true";
    const source = page.querySelector<HTMLTextAreaElement>("[data-indicator-source]");
    const file = page.querySelector<HTMLInputElement>("[data-indicator-import]");
    page.querySelector<HTMLButtonElement>("[data-indicator-import-trigger]")?.addEventListener("click", () => file?.click());
    file?.addEventListener("change", () => {
      const selected = file.files?.[0];
      file.value = "";
      if (selected && source) void selected.text().then((text) => { source.value = text; });
    });
    const chartElement = page.querySelector<HTMLElement>("[data-indicator-chart]");
    const definition = page.querySelector<HTMLSelectElement>("[data-indicator-chart-definition]");
    const instrument = page.querySelector<HTMLSelectElement>("[data-indicator-chart-instrument]");
    if (!chartElement || !definition || !instrument) return;
    const load = async () => {
      removeChart(chartElement);
      chartElement.replaceChildren();
      const agentKey = chartElement.dataset.agentKey;
      if (!agentKey || !definition.value || !instrument.value) return;
      const response = await fetch(`/agents/${encodeURIComponent(agentKey)}/indicators/chart-data?indicator_id=${encodeURIComponent(definition.value)}&instrument_id=${encodeURIComponent(instrument.value)}`);
      if (!response.ok) { chartElement.textContent = "No successful run is available for this selection."; return; }
      const data = await response.json() as { candles: Array<{ time: UTCTimestamp; open: number; high: number; low: number; close: number }>; plots: Record<string, Array<number | null>> };
      if (data.candles.length === 0) { chartElement.textContent = "No candle data is available."; return; }
      const chart = createChart(chartElement, { width: chartElement.clientWidth, height: 360, layout: { background: { color: "#09090b" }, textColor: "#a1a1aa" }, grid: { vertLines: { color: "#27272a" }, horzLines: { color: "#27272a" } } });
      chart.addSeries(CandlestickSeries).setData(data.candles);
      Object.entries(data.plots).forEach(([title, values]) => chart.addSeries(LineSeries, { title }).setData(values.flatMap((value, index) => value === null ? [] : [{ time: data.candles[index]?.time, value }])));
      const observer = new ResizeObserver(() => chart.applyOptions({ width: chartElement.clientWidth }));
      observer.observe(chartElement);
      charts.set(chartElement, { remove: () => { observer.disconnect(); chart.remove(); } });
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
