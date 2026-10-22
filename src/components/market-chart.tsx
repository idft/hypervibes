import { useEffect, useRef, useState, type Ref } from "react";
import { createPortal } from "react-dom";
import { CandlestickSeries, ColorType, HistogramSeries, createChart, type IChartApi, type ISeriesApi, type Logical, type UTCTimestamp } from "lightweight-charts";
import { mergeChartCandles, intervalMillis, type ChartCandle, type ChartSnapshot } from "../hyperliquid/chart-contracts.js";
import type { ChartCapture, ChartDrawing, ChartLayout, ChartPoint, DrawingKind } from "../charts/contracts.js";
import { useQueryClient } from "@tanstack/react-query";
import { chartHistoryQueryOptions } from "../web/chart-queries.js";
import { ChartDrawings, type ChartProjection } from "./chart-drawings.js";
import { defaultChartOverlays, type ChartOverlayVisibility } from "../charts/trading-contracts.js";
import { createTradingOverlays, type FillHover } from "../charts/trading-overlays.js";
import { useChartAccount } from "./use-chart-account.js";
import { useChartIndicators } from "./use-chart-indicators.js";
import { createIndicatorRenderer } from "../charts/indicator-renderer.js";
import type { ChartIndicatorEvidence, IndicatorResult } from "../indicators/contracts.js";

export interface MarketChartHandle { capture: (layout: ChartLayout, selectedDrawingId: string, image: boolean) => Promise<ChartCapture> }
interface ActiveChart { chart: IChartApi; price: ISeriesApi<"Candlestick">; candles: ChartCandle[]; status: ChartSnapshot["status"]; overlays: ReturnType<typeof createTradingOverlays>; indicators: ReturnType<typeof createIndicatorRenderer>; evidence: { configuration: ChartIndicatorEvidence["configuration"]; result?: IndicatorResult; status: ChartIndicatorEvidence["status"]; error?: string }[] }
export function MarketChart({ id, layout, tool, selected, disabled, statusHost, overlayVisibility = defaultChartOverlays, onSelect, onSave, onDrawn, onEdit, ref }: {
  id: string; layout: ChartLayout; tool: DrawingKind | "select"; selected: string; disabled: boolean; statusHost: HTMLElement | null; overlayVisibility?: ChartOverlayVisibility;
  onSelect: (id: string) => void; onSave: (drawing: ChartDrawing) => Promise<number | undefined>; onDrawn: () => void; onEdit: () => void; ref: Ref<MarketChartHandle>;
}) {
  const client = useQueryClient();
  const indicatorData = useChartIndicators(id, layout);
  const [generated, setGenerated] = useState<{ id: string; text: string } | null>(null);
  const host = useRef<HTMLDivElement>(null), svg = useRef<SVGSVGElement>(null), active = useRef<ActiveChart | null>(null);
  const [status, setStatus] = useState<ChartSnapshot["status"]>("connecting");
  const [error, setError] = useState("");
  const [historyStatus, setHistoryStatus] = useState("Loading candles…");
  const [retry, setRetry] = useState(0), [frame, setFrame] = useState(0);
  const [hover, setHover] = useState<FillHover | null>(null);
  const { coin, interval } = layout.document;
  useEffect(() => {
    const element = host.current;
    if (!element) return;
    let disposed = false, loading = false, initialized = false, hasMore = true, historyFailed = false, raf = 0;
    const chart = createChart(element, {
      autoSize: true,
      layout: { background: { type: ColorType.Solid, color: "#0d0d10" }, textColor: "#a1a1aa", attributionLogo: true },
      grid: { vertLines: { color: "#1c1c22" }, horzLines: { color: "#1c1c22" } },
      rightPriceScale: { borderColor: "#27272a" }, timeScale: { timeVisible: true, secondsVisible: false, borderColor: "#27272a" },
    });
    const price = chart.addSeries(CandlestickSeries, { upColor: "#34d399", downColor: "#f87171", borderVisible: false, wickUpColor: "#34d399", wickDownColor: "#f87171" });
    price.priceScale().applyOptions({ scaleMargins: { top: 0.06, bottom: 0.25 } });
    const volume = chart.addSeries(HistogramSeries, { priceFormat: { type: "volume" }, priceScaleId: "volume" });
    volume.priceScale().applyOptions({ scaleMargins: { top: 0.8, bottom: 0 }, visible: false });
    const overlays = createTradingOverlays(chart, price, setHover);
    const indicators = createIndicatorRenderer(chart, price);
    const state: ActiveChart = { chart, price, candles: [], status: "connecting", overlays, indicators, evidence: [] }; 
    const generatedHover: Parameters<IChartApi["subscribeCrosshairMove"]>[0] = (event) => { const id = event.hoveredInfo?.objectId; const text = typeof id === "string" ? indicators.describe(id) : undefined; setGenerated(typeof id === "string" && text !== undefined ? { id, text } : null); };
    chart.subscribeCrosshairMove(generatedHover);
    active.current = state;
    const redraw = () => {
      if (raf || disposed) return;
      raf = requestAnimationFrame(() => { raf = 0; if (!disposed) setFrame((n) => n + 1); });
    };
    const draw = () => {
      const close = state.candles.at(-1)?.close;
      if (close && close > 0) {
        const precision = Math.min(12, Math.max(2, 5 - Math.floor(Math.log10(close))));
        price.applyOptions({ priceFormat: { type: "price", precision, minMove: 10 ** -precision } });
      }
      price.setData(state.candles.map((c) => ({ ...c, time: c.time as UTCTimestamp })));
      volume.setData(state.candles.map((c) => ({ time: c.time as UTCTimestamp, value: c.volume, color: c.close >= c.open ? "#34d39960" : "#f8717160" })));
      redraw();
    };
    const load = async (before?: number) => {
      if (disposed || loading || !hasMore) return;
      loading = true; setHistoryStatus("Loading candles…");
      try {
        const result = await client.fetchQuery(chartHistoryQueryOptions(id, coin, interval, before));
        if (disposed) return;
        const range = chart.timeScale().getVisibleLogicalRange(), oldest = state.candles[0]?.time;
        state.candles = mergeChartCandles(result.candles, state.candles);
        const added = oldest === undefined ? 0 : state.candles.filter((c) => c.time < oldest).length;
        hasMore = result.hasMore && (before === undefined || added > 0);
        draw();
        if (!initialized && state.candles.length) chart.timeScale().setVisibleLogicalRange({ from: Math.max(0, state.candles.length - 120), to: state.candles.length + 3 });
        else if (range && added) chart.timeScale().setVisibleLogicalRange({ from: range.from + added, to: range.to + added });
        initialized = true; historyFailed = false;
        setHistoryStatus(hasMore ? "" : "No older candles available from Hyperliquid.");
      } catch (failure) {
        if (!disposed) { historyFailed = true; setError(failure instanceof Error ? failure.message : "Candle history is unavailable."); setHistoryStatus("History unavailable. Retry to reload."); }
      } finally { loading = false; }
    };
    const onRange = (range: { from: number; to: number } | null) => {
      redraw();
      if (range && range.from < 50 && initialized && !historyFailed && state.candles.length) void load(state.candles[0]!.time * 1000);
    };
    chart.timeScale().subscribeVisibleLogicalRangeChange(onRange);
    chart.timeScale().subscribeSizeChange(redraw);
    chart.subscribeCrosshairMove(redraw);
    element.addEventListener("pointermove", redraw, { passive: true });
    element.addEventListener("wheel", redraw, { passive: true });
    const source = new EventSource(`/api/agents/${id}/chart/events?${new URLSearchParams({ coin, interval })}`);
    source.addEventListener("chart", (event) => {
      if (disposed) return;
      const snapshot = JSON.parse((event as MessageEvent).data) as ChartSnapshot;
      state.status = snapshot.status; setStatus(snapshot.status);
      const previousLength = state.candles.length;
      state.candles = mergeChartCandles(state.candles, snapshot.candles);
      if (!initialized || state.candles.length !== previousLength) draw();
      else {
        for (const c of snapshot.candles) {
          price.update({ ...c, time: c.time as UTCTimestamp }, true);
          volume.update({ time: c.time as UTCTimestamp, value: c.volume, color: c.close >= c.open ? "#34d39960" : "#f8717160" }, true);
        }
        redraw();
      }
    });
    const interrupted = () => { if (!disposed) { state.status = "reconnecting"; setStatus("reconnecting"); } };
    source.onerror = interrupted;
    for (const name of ["shutdown", "unavailable"]) source.addEventListener(name, interrupted);
    source.addEventListener("stream-error", () => { source.close(); interrupted(); setError("Chart stream unavailable. Retry to reconnect."); });
    source.addEventListener("unauthorized", () => { source.close(); window.location.assign("/login"); });
    setError(""); void load();
    return () => {
      disposed = true; active.current = null; cancelAnimationFrame(raf); source.close();
      element.removeEventListener("pointermove", redraw); element.removeEventListener("wheel", redraw);
      chart.timeScale().unsubscribeVisibleLogicalRangeChange(onRange); chart.timeScale().unsubscribeSizeChange(redraw); chart.unsubscribeCrosshairMove(redraw); chart.unsubscribeCrosshairMove(generatedHover); indicators.close(); overlays.close(); chart.remove();
    };
  }, [client, id, coin, interval, retry]);
  const state = active.current;
  const candles = state?.candles;
  const step = intervalMillis[interval] / 1000;
  const account = useChartAccount(id, coin, (candles?.[0]?.time ?? 0) * 1000, candles?.length ? (candles.at(-1)!.time + step) * 1000 : 0, overlayVisibility);
  useEffect(() => {
    if (state && state === active.current && candles) state.overlays.trades(overlayVisibility.trades ? account.fills : [], candles, step);
  }, [state, candles, step, account.fills, overlayVisibility.trades]);
  useEffect(() => {
    if (state && state === active.current) state.overlays.account(coin, account.snapshot, overlayVisibility, account.interrupted);
  }, [state, coin, account.snapshot, account.interrupted, overlayVisibility]);
  useEffect(() => {
    if (!state || state !== active.current || !candles?.length) return;
    const configurations = layout.document.indicators;
    for (const evidence of state.evidence) if (!configurations.some((i) => i.id === evidence.configuration.id && !i.hidden)) state.indicators.remove(evidence.configuration.id);
    state.evidence = configurations.filter((i) => !i.hidden).map((configuration) => {
      const item = indicatorData.data?.instances.find((item) => item.instanceId === configuration.id);
      if (item?.result) {
        try { state.indicators.apply(configuration, item.result, candles); }
        catch (error) { state.indicators.remove(configuration.id); return { configuration, status: "error" as const, error: error instanceof Error ? error.message : "Unable to render indicator" }; }
      } else state.indicators.remove(configuration.id);
      return { configuration, result: item?.result, status: item?.error ? "error" as const : indicatorData.status, error: item?.error ?? indicatorData.error };
    });
  }, [state, candles, layout.document.indicators, indicatorData.data, indicatorData.status, indicatorData.error]);
  // Frame advances on chart range, scale, pointer, size or candle updates—not on every animation frame.
  const projection = state ? projectChart(state, intervalMillis[interval] / 1000) : null;
  const handle: MarketChartHandle = { capture: async (frozenLayout, selectedDrawingId, includeImage) => {
    const current = active.current;
    if (!current?.candles.length) throw new Error("Wait for chart candles before sending chart context.");
    const scale = current.chart.timeScale(), range = scale.getVisibleLogicalRange();
    if (!range) throw new Error("Chart viewport is not ready.");
    const step = intervalMillis[frozenLayout.document.interval] / 1000;
    const from = timeAt(current.candles, range.from, step), to = timeAt(current.candles, range.to, step);
    const visible = current.candles.filter((c) => c.time >= from && c.time <= to);
    if (visible.length > 1500) throw new Error("Zoom in to at most 1,500 visible candles before sending chart context.");
    const capture: ChartCapture = { layoutId: frozenLayout.id, revision: frozenLayout.revision, capturedAt: Date.now(), status: current.status, range: { from: Math.max(0, from), to }, candles: visible, indicators: current.evidence.map(({ configuration, result, status, error }): ChartIndicatorEvidence => ({ instanceId: configuration.id, indicatorId: configuration.indicatorId, configuration, status, ...(error ? { error: error.slice(0, 1000) } : {}), ...(result ? { result: { version: result.version, sourceHash: result.sourceHash, engine: result.engine, wrapperVersion: result.wrapperVersion, resultHash: result.resultHash, resolvedInputs: result.resolvedInputs, inputSchemaHash: result.inputSchemaHash, computedAt: result.computedAt, end: result.end, dataset: result.dataset, coverage: result.coverage, sampleScope: "partial-visible" as const, sample: result.values.filter((p) => p.time >= from && p.time <= to).slice(-20), objects: [...result.markers.filter((m) => m.displayTime >= from && m.displayTime <= to).map((m) => ({ id: m.id, kind: m.kind, time: m.time, text: m.text ?? m.character })), ...result.drawings.map((d) => ({ id: d.id, kind: d.kind, time: d.kind === "label" ? d.at.time : d.from.time, text: d.kind === "line" ? undefined : d.text })).filter((d) => d.time >= from && d.time <= to)].slice(-20) } } : {}) })), ...(generated ? { selectedGeneratedObjectId: generated.id } : {}), ...(selectedDrawingId ? { selectedDrawingId } : {}) };
    if (includeImage) capture.image = await screenshot(current.chart.takeScreenshot(), svg.current);
    // Keep room for the user message and Start's envelope under the host's 1 MiB request limit.
    if (new TextEncoder().encode(JSON.stringify(capture)).byteLength > 900_000) throw new Error("Chart context is too large. Zoom in or disable Include chart image.");
    return capture;
  } };
  useEffect(() => {
    if (typeof ref === "function") { ref(handle); return () => { ref(null); }; }
    if (ref) { ref.current = handle; return () => { ref.current = null; }; }
  });
  const indicatorMessages = layout.document.indicators.some((i) => !i.hidden)
    ? [indicatorData.status !== "current" ? `Indicators: ${indicatorData.status === "stale" ? "waiting for closed candle" : indicatorData.status}` : "", indicatorData.error, ...(indicatorData.data?.instances.flatMap((i) => i.error ? [i.error] : []) ?? [])].filter((message): message is string => Boolean(message))
    : [];
  const statusMessages = [status === "reconnecting" ? "Reconnecting… Data may be stale." : "", historyStatus, error, ...indicatorMessages, ...account.messages].filter(Boolean);
  const indicatorFailed = indicatorMessages.length > 0 && Boolean(indicatorData.error || indicatorData.data?.instances.some((i) => i.error));
  const needsAttention = Boolean(error || indicatorFailed || account.canRetry);
  const loading = status !== "live" || needsAttention || indicatorMessages.length > 0 || account.messages.length > 0 || historyStatus === "Loading candles…";
  const disconnected = status === "reconnecting";
  const connectionLabel = disconnected ? "Disconnected" : loading ? "Loading" : "Connected";
  return <div className="flex min-h-0 flex-1 flex-col gap-2" data-frame={frame}>
    {statusHost && createPortal(<details className="relative">
      <summary className="flex cursor-pointer list-none items-center gap-2 whitespace-nowrap text-xs text-zinc-400 [&::-webkit-details-marker]:hidden" title={statusMessages.join("\n") || "All chart data is up to date. Click for details."}>
        <span role="status" aria-label="Chart connection" title={statusMessages.join("\n") || undefined} className="inline-flex items-center gap-2">
          <span aria-hidden="true" className={`inline-block h-2 w-2 rounded-full ${disconnected ? "bg-red-400" : loading ? "bg-amber-400" : "bg-emerald-400"}`} />
          {connectionLabel}
          {statusMessages.length > 0 && <span className="sr-only"> · {statusMessages.join(" · ")}</span>}
        </span>
      </summary>
      <div aria-label="Chart data status" className="absolute right-0 top-full z-50 mt-2 w-80 max-w-[calc(100vw-2rem)] space-y-3 rounded-xl border border-zinc-700 bg-zinc-950 p-4 text-sm text-zinc-300 shadow-xl">
        <h3 className="font-medium text-white">Chart status</h3>
        <p>Market stream: {status === "live" ? "Connected" : status === "connecting" ? "Connecting…" : "Disconnected"}</p>
        {statusMessages.length ? <ul className="space-y-2">{statusMessages.map((message, index) => <li key={index}>{message}</li>)}</ul> : <p>All chart data is up to date.</p>}
        <div className="flex flex-wrap gap-3">
          {(error || status === "reconnecting") && <button type="button" className="cursor-pointer underline" onClick={() => { setStatus("connecting"); setRetry((n) => n + 1); }}>Retry chart</button>}
          {indicatorMessages.length > 0 && <button type="button" className="cursor-pointer underline" onClick={indicatorData.retry}>Retry indicators</button>}
          {account.canRetry && <button type="button" className="cursor-pointer underline" onClick={account.retry}>Retry account data</button>}
        </div>
      </div>
    </details>, statusHost)}
    <div className="relative min-h-64 flex-1 overflow-hidden rounded-xl border border-zinc-800" aria-label={`${coin} ${interval} candlestick and volume chart`}>
      {/* Contain the library's positive canvas z-indices below the drawing overlay. */}
      <div ref={host} className="absolute inset-0 z-0" />
      {generated && <div role="tooltip" className="pointer-events-none absolute left-2 top-2 z-20 max-w-[calc(100%-5rem)] rounded bg-zinc-950/85 px-2 py-1 text-xs text-zinc-300">Read-only Pine object: {generated.text}</div>}
      {hover && overlayVisibility.trades && <div role="tooltip" aria-label="Trade fill" className="pointer-events-none absolute z-20 max-w-72 rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-xs text-zinc-200 shadow-xl" style={{ left: Math.max(0, Math.min(hover.x + 12, (projection?.width ?? 300) - 280)), top: Math.max(0, Math.min(hover.y + 12, (projection?.height ?? 100) - 70)) }}>
        <p className={hover.fill.side === "Buy" ? "text-emerald-400" : "text-red-400"}>{hover.fill.side} · {hover.fill.size} {coin} @ {hover.fill.price}</p>
        <time dateTime={new Date(hover.fill.time).toISOString()}>{new Date(hover.fill.time).toLocaleString()}</time>
      </div>}
      {projection && <ChartDrawings key={`${layout.id}/${coin}/${interval}/${tool}`} document={layout.document} revision={layout.revision} projection={projection} tool={tool} selected={selected} disabled={disabled} svgRef={svg} onSelect={onSelect} onSave={onSave} onDrawn={onDrawn} onEdit={onEdit} />}
    </div>
  </div>;
}
function timeAt(candles: ChartCandle[], logical: number, step: number): number {
  if (!candles.length) return 0;
  const floor = Math.floor(logical), a = candles[Math.max(0, Math.min(candles.length - 1, floor))]!;
  if (logical < 0) return candles[0]!.time + logical * step;
  if (floor >= candles.length - 1) return a.time + (logical - candles.length + 1) * step;
  return a.time + (candles[floor + 1]!.time - a.time) * (logical - floor);
}
function logicalAt(candles: ChartCandle[], time: number, step: number): number {
  if (!candles.length) return 0;
  if (time < candles[0]!.time) return (time - candles[0]!.time) / step;
  let lo = 0, hi = candles.length - 1;
  while (lo < hi) { const mid = Math.ceil((lo + hi) / 2); if (candles[mid]!.time <= time) lo = mid; else hi = mid - 1; }
  return lo + (time - candles[lo]!.time) / (candles[lo + 1] ? candles[lo + 1]!.time - candles[lo]!.time : step);
}
function projectChart(state: ActiveChart, step: number): ChartProjection {
  const { chart, price, candles } = state, scale = chart.timeScale();
  const coordinateAt = (logical: number) => {
    // Lightweight Charts accepts only integer indices here; fractional anchors otherwise become x=0.
    const index = Math.floor(logical), fraction = logical - index;
    const left = scale.logicalToCoordinate(index as Logical);
    if (left === null || fraction === 0) return left;
    const right = scale.logicalToCoordinate((index + 1) as Logical);
    return right === null ? null : left + (right - left) * fraction;
  };
  return { width: Math.max(0, chart.chartElement().clientWidth - chart.priceScale("right").width()), height: Math.max(0, chart.panes()[0]?.getHeight() ?? chart.chartElement().clientHeight - scale.height()),
    x: (time) => coordinateAt(logicalAt(candles, time, step)), y: (value) => price.priceToCoordinate(value),
    point: (x, y, snap): ChartPoint | null => {
      const nearest = scale.coordinateToLogical(x), value = price.coordinateToPrice(y);
      if (nearest === null || value === null || value <= 0) return null;
      // The inverse API rounds to a bar. Recover the sub-bar position before snapping or saving.
      const index = Math.floor(nearest), left = coordinateAt(index), right = coordinateAt(index + 1);
      if (left === null || right === null || left === right) return null;
      const logical = index + (x - left) / (right - left);
      const candle = candles[Math.round(logical)];
      if (snap && candle) {
        const nearest = [candle.open, candle.high, candle.low, candle.close].reduce((a, b) => Math.abs(a - value) < Math.abs(b - value) ? a : b);
        return { time: candle.time, price: nearest };
      }
      return { time: Math.max(0, timeAt(candles, logical, step)), price: value };
    } };
}
async function screenshot(canvas: HTMLCanvasElement, svg: SVGSVGElement | null): Promise<string> {
  const context = canvas.getContext("2d");
  if (!context) throw new Error("Chart screenshot is unavailable in this browser.");
  if (svg) {
    const clone = svg.cloneNode(true) as SVGSVGElement;
    clone.querySelectorAll("[data-handle]").forEach((node) => node.remove());
    const url = URL.createObjectURL(new Blob([new XMLSerializer().serializeToString(clone)], { type: "image/svg+xml" }));
    try {
      const image = new Image(); image.src = url; await image.decode();
      const ratio = canvas.width / Math.max(1, canvas.clientWidth || canvas.width / window.devicePixelRatio);
      context.drawImage(image, 0, 0, Number(svg.getAttribute("width")) * ratio, Number(svg.getAttribute("height")) * ratio);
    } finally { URL.revokeObjectURL(url); }
  }
  const output = document.createElement("canvas");
  for (const max of [1600, 1000, 700]) {
    const ratio = Math.min(1, max / canvas.width, 1000 / canvas.height);
    output.width = Math.round(canvas.width * ratio); output.height = Math.round(canvas.height * ratio);
    output.getContext("2d")!.drawImage(canvas, 0, 0, output.width, output.height);
    const data = output.toDataURL("image/png").split(",")[1]!;
    if (data.length <= 700_000) return data;
  }
  throw new Error("Chart image is too large. Zoom in or disable Include chart image.");
}
