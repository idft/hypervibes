import {
  CandlestickSeries,
  LineSeries,
  createChart,
  createSeriesMarkers,
  type SeriesMarker,
  type UTCTimestamp,
} from "lightweight-charts";

type ChartHandle = { remove: () => void };
type IndicatorColor = { red: number; green: number; blue: number; transparency: number };
type PlotshapeMarker = {
  kind: "plotshape";
  time: UTCTimestamp;
  title: string;
  text: string;
  style: string;
  location: string;
  color: IndicatorColor | null;
  text_color: IndicatorColor | null;
  size: string;
  price: number | null;
};
type PlotcharMarker = {
  kind: "plotchar";
  time: UTCTimestamp;
  title: string;
  character: string;
  text: string;
  location: string;
  color: IndicatorColor | null;
  text_color: IndicatorColor | null;
  size: string;
  price: number | null;
};
type PlotarrowMarker = {
  kind: "plotarrow";
  time: UTCTimestamp;
  title: string;
  direction: "up" | "down";
  value: number;
  color: IndicatorColor | null;
};
type IndicatorMarker = PlotshapeMarker | PlotcharMarker | PlotarrowMarker;
type ChartData = {
  indicator: { overlay: boolean };
  candles: Array<{ time: UTCTimestamp; open: number; high: number; low: number; close: number }>;
  plots: Array<{ title: string; values: Array<{ time: UTCTimestamp; value: number }> }>;
  markers: IndicatorMarker[];
};

const MAX_IMPORT_BYTES = 256 * 1024;
const charts = new WeakMap<HTMLElement, ChartHandle>();
const NEUTRAL_MARKER_COLOR = "#a1a1aa";

function markerColor(color: IndicatorColor | null): string {
  if (!color) return NEUTRAL_MARKER_COLOR;
  const transparency = Math.min(100, Math.max(0, color.transparency));
  return `rgba(${color.red}, ${color.green}, ${color.blue}, ${1 - transparency / 100})`;
}

function markerSize(size: string): number {
  switch (size) {
    case "tiny": return 0.5;
    case "small": return 0.75;
    case "large": return 1.5;
    case "huge": return 2;
    default: return 1;
  }
}

function markerPosition(location: string, price: number | null) {
  if (location === "absolute") {
    if (price === null) throw new Error("absolute indicator marker is missing its price");
    return { position: "atPriceMiddle" as const, price };
  }
  return { position: location === "abovebar" || location === "top" ? "aboveBar" as const : "belowBar" as const };
}

function plotshapeShape(style: string): "circle" | "square" | "arrowUp" | "arrowDown" {
  if (style === "triangleup" || style === "arrowup" || style === "labelup") return "arrowUp";
  if (style === "triangledown" || style === "arrowdown" || style === "labeldown") return "arrowDown";
  if (style === "square" || style === "diamond") return "square";
  return "circle";
}

function chartMarker(marker: IndicatorMarker): SeriesMarker<UTCTimestamp> {
  if (marker.kind === "plotarrow") {
    return {
      time: marker.time,
      position: marker.direction === "up" ? "belowBar" : "aboveBar",
      shape: marker.direction === "up" ? "arrowUp" : "arrowDown",
      color: markerColor(marker.color),
      size: 1,
    };
  }
  const color = markerColor(marker.color ?? marker.text_color);
  if (marker.kind === "plotshape") {
    return {
      time: marker.time,
      shape: plotshapeShape(marker.style),
      color,
      size: markerSize(marker.size),
      ...markerPosition(marker.location, marker.price),
      ...(marker.text ? { text: marker.text } : {}),
    };
  }
  const text = [marker.character, marker.text].filter(Boolean).join(" ");
  return {
    time: marker.time,
    shape: "circle",
    color,
    size: markerSize(marker.size),
    ...markerPosition(marker.location, marker.price),
    ...(text ? { text } : {}),
  };
}

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
    const timeframeList = page.querySelector<HTMLElement>("[data-indicator-timeframe-list]");
    const addTimeframe = page.querySelector<HTMLButtonElement>("[data-indicator-timeframe-add]");
    const syncTimeframeRows = () => {
      const rows = [...(timeframeList?.querySelectorAll<HTMLElement>("[data-indicator-timeframe-row]") ?? [])];
      rows.forEach((row) => {
        const remove = row.querySelector<HTMLButtonElement>("[data-indicator-timeframe-remove]");
        if (remove) remove.disabled = rows.length === 1;
      });
      if (addTimeframe) addTimeframe.disabled = rows.length >= 8;
    };
    timeframeList?.addEventListener("click", (event) => {
      const remove = (event.target as Element).closest<HTMLButtonElement>("[data-indicator-timeframe-remove]");
      if (!remove || timeframeList.querySelectorAll("[data-indicator-timeframe-row]").length === 1) return;
      remove.closest("[data-indicator-timeframe-row]")?.remove();
      syncTimeframeRows();
    });
    addTimeframe?.addEventListener("click", () => {
      if (!timeframeList || timeframeList.querySelectorAll("[data-indicator-timeframe-row]").length >= 8) return;
      const row = document.createElement("div");
      row.dataset.indicatorTimeframeRow = "";
      row.className = "flex gap-2";
      row.innerHTML = '<input required name="timeframe" placeholder="1h" class="min-w-0 flex-1 rounded-xl border border-zinc-800 bg-zinc-950 px-3 py-2 text-zinc-100"><button type="button" data-indicator-timeframe-remove class="cursor-pointer rounded-full border border-zinc-700 px-3 text-xs text-zinc-300 hover:text-white">Remove</button>';
      timeframeList.append(row);
      row.querySelector<HTMLInputElement>('input[name="timeframe"]')?.focus();
      syncTimeframeRows();
    });
    syncTimeframeRows();

    const timeframeSelector = page.querySelector<HTMLSelectElement>("[data-indicator-timeframe-selector]");
    timeframeSelector?.addEventListener("change", () => {
      page.querySelectorAll<HTMLElement>("[data-indicator-timeframe-panel]").forEach((panel) => {
        panel.hidden = panel.dataset.indicatorTimeframePanel !== timeframeSelector.value;
      });
    });
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
      const timeframe = chartElement.dataset.timeframe;
      if (!agentKey || !indicatorId || !instrumentId || !timeframe) {
        chartElement.textContent = "Chart data is unavailable.";
        return;
      }
      try {
        const response = await fetch(`/agents/${encodeURIComponent(agentKey)}/indicators/chart-data?indicator_id=${encodeURIComponent(indicatorId)}&instrument_id=${encodeURIComponent(instrumentId)}&timeframe=${encodeURIComponent(timeframe)}`);
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
        const candleSeries = chart.addSeries(CandlestickSeries);
        candleSeries.setData(data.candles);
        const plotPane = data.indicator.overlay ? 0 : chart.addPane().paneIndex();
        data.plots.forEach((plot) => chart.addSeries(LineSeries, { title: plot.title }, plotPane).setData(plot.values));
        if (data.markers.length > 0) createSeriesMarkers(candleSeries, data.markers.map(chartMarker));
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
