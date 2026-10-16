import { useEffect, useRef, useState } from "react";
import { CandlestickSeries, ColorType, HistogramSeries, createChart, type UTCTimestamp } from "lightweight-charts";
import { chartIntervals, isChartInterval, mergeChartCandles, type ChartCandle, type ChartSelection, type ChartSnapshot } from "../hyperliquid/chart-contracts.js";
import { getChartHistory } from "../web/charts.js";

const preferenceKey = (id: string) => `hypervibes:chart:${id}`;
const defaults: ChartSelection = { coin: "BTC", interval: "1h" };

export function AgentChartPage({ id, instruments }: { id: string; instruments: string[] }) {
  const [selection, setSelection] = useState<ChartSelection | null>(null);
  useEffect(() => {
    let saved = defaults;
    try {
      const value: unknown = JSON.parse(localStorage.getItem(preferenceKey(id)) ?? "null");
      if (value && typeof value === "object" && "coin" in value && "interval" in value
        && typeof value.coin === "string" && instruments.includes(value.coin) && typeof value.interval === "string" && isChartInterval(value.interval)) {
        saved = { coin: value.coin, interval: value.interval };
      }
    } catch { /* Storage is optional, including in privacy mode. */ }
    setSelection({ ...saved, coin: instruments.includes(saved.coin) ? saved.coin : instruments[0] ?? "BTC" });
  }, [id, instruments]);
  function select(value: ChartSelection) {
    setSelection(value);
    try { localStorage.setItem(preferenceKey(id), JSON.stringify(value)); } catch { /* Optional browser preference. */ }
  }
  if (!instruments.length) return <p role="alert" className="text-amber-200">No perpetual instruments are available.</p>;
  return <section className="relative flex w-full min-w-0 flex-col gap-4" aria-label="Trading chart">
    <h2 className="sr-only">Chart</h2>
    {selection && <>
      <div className="flex flex-wrap items-center gap-4 pr-36">
        <label className="flex items-center gap-2 text-sm text-zinc-400">Instrument
          <select aria-label="Instrument" value={selection.coin} onChange={(event) => select({ ...selection, coin: event.target.value })}
            className="cursor-pointer rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-white">
            {instruments.map((coin) => <option key={coin} value={coin}>{coin}-PERP</option>)}
          </select>
        </label>
        <label className="flex items-center gap-2 text-sm text-zinc-400">Timeframe
          <select aria-label="Timeframe" value={selection.interval} onChange={(event) => { if (isChartInterval(event.target.value)) select({ ...selection, interval: event.target.value }); }}
            className="cursor-pointer rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-white">
            {chartIntervals.map((interval) => <option key={interval} value={interval}>{interval}</option>)}
          </select>
        </label>
      </div>
      <MarketChart key={`${id}/${selection.coin}/${selection.interval}`} id={id} selection={selection} />
    </>}
  </section>;
}

function MarketChart({ id, selection }: { id: string; selection: ChartSelection }) {
  const host = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<ChartSnapshot["status"]>("connecting");
  const [error, setError] = useState("");
  const [historyStatus, setHistoryStatus] = useState("Loading candles…");
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!host.current) return;
    let disposed = false;
    let loading = false;
    let initialized = false;
    let hasMore = true;
    let historyFailed = false;
    let values: ChartCandle[] = [];
    const chart = createChart(host.current, {
      autoSize: true,
      layout: { background: { type: ColorType.Solid, color: "#0d0d10" }, textColor: "#a1a1aa", attributionLogo: true },
      grid: { vertLines: { color: "#1c1c22" }, horzLines: { color: "#1c1c22" } },
      rightPriceScale: { borderColor: "#27272a" },
      timeScale: { timeVisible: true, secondsVisible: false, borderColor: "#27272a" },
    });
    const price = chart.addSeries(CandlestickSeries, { upColor: "#34d399", downColor: "#f87171", borderVisible: false, wickUpColor: "#34d399", wickDownColor: "#f87171" });
    price.priceScale().applyOptions({ scaleMargins: { top: 0.06, bottom: 0.25 } });
    const volume = chart.addSeries(HistogramSeries, { priceFormat: { type: "volume" }, priceScaleId: "volume" });
    volume.priceScale().applyOptions({ scaleMargins: { top: 0.8, bottom: 0 }, visible: false });
    const draw = () => {
      const close = values.at(-1)?.close;
      if (close && close > 0) {
        const precision = Math.min(12, Math.max(2, 5 - Math.floor(Math.log10(close))));
        price.applyOptions({ priceFormat: { type: "price", precision, minMove: 10 ** -precision } });
      }
      price.setData(values.map((candle) => ({ ...candle, time: candle.time as UTCTimestamp })));
      volume.setData(values.map((candle) => ({ time: candle.time as UTCTimestamp, value: candle.volume, color: candle.close >= candle.open ? "#34d39960" : "#f8717160" })));
    };
    const load = async (before?: number) => {
      if (disposed || loading || !hasMore) return;
      loading = true;
      setHistoryStatus("Loading candles…");
      try {
        const result = await getChartHistory({ data: { id, ...selection, ...(before === undefined ? {} : { before }) } });
        if (disposed) return;
        if (!result.ok) throw new Error(result.error);
        const range = chart.timeScale().getVisibleLogicalRange();
        const oldest = values[0]?.time;
        // Live snapshots win over HTTP when both include the current candle.
        values = mergeChartCandles(result.data.candles, values);
        const added = oldest === undefined ? 0 : values.filter((candle) => candle.time < oldest).length;
        hasMore = result.data.hasMore && (before === undefined || added > 0);
        draw();
        if (!initialized && values.length) {
          chart.timeScale().setVisibleLogicalRange({ from: Math.max(0, values.length - 120), to: values.length + 3 });
        } else if (range && added) chart.timeScale().setVisibleLogicalRange({ from: range.from + added, to: range.to + added });
        initialized = true;
        historyFailed = false;
        setHistoryStatus(hasMore ? "" : "No older candles available from Hyperliquid.");
      } catch (failure) {
        if (!disposed) {
          historyFailed = true;
          setError(failure instanceof Error ? failure.message : "Candle history is unavailable.");
          setHistoryStatus("History unavailable. Retry to reload.");
        }
      } finally { loading = false; }
    };
    const onRange = (range: { from: number; to: number } | null) => {
      if (range && range.from < 50 && initialized && !historyFailed && values.length) void load(values[0]!.time * 1000);
    };
    chart.timeScale().subscribeVisibleLogicalRangeChange(onRange);
    const query = new URLSearchParams({ ...selection });
    const source = new EventSource(`/api/agents/${id}/chart/events?${query}`);
    source.addEventListener("chart", (event) => {
      if (disposed) return;
      const snapshot = JSON.parse((event as MessageEvent).data) as ChartSnapshot;
      setStatus(snapshot.status);
      const previousLength = values.length;
      values = mergeChartCandles(values, snapshot.candles);
      if (!initialized || values.length !== previousLength) draw();
      else {
        // Historical points can be corrected on reconnect; update them as well.
        for (const candle of snapshot.candles) {
          price.update({ ...candle, time: candle.time as UTCTimestamp }, true);
          volume.update({ time: candle.time as UTCTimestamp, value: candle.volume, color: candle.close >= candle.open ? "#34d39960" : "#f8717160" }, true);
        }
      }
    });
    const interrupted = () => { if (!disposed) setStatus("reconnecting"); };
    source.onerror = interrupted;
    source.addEventListener("shutdown", interrupted);
    source.addEventListener("unavailable", interrupted);
    source.addEventListener("stream-error", () => { source.close(); setStatus("reconnecting"); setError("Chart stream unavailable. Retry to reconnect."); });
    source.addEventListener("unauthorized", () => { source.close(); window.location.assign("/login"); });
    setError("");
    void load();
    return () => {
      disposed = true;
      source.close();
      chart.timeScale().unsubscribeVisibleLogicalRangeChange(onRange);
      chart.remove();
    };
  }, [id, selection, retry]);
  return <>
    <p role="status" className="absolute right-0 top-2 flex items-center gap-2 text-xs text-zinc-400" title={status === "reconnecting" ? "Reconnecting… Data may be stale." : undefined}>
      <span aria-hidden="true" className={`h-2 w-2 rounded-full ${status === "live" ? "bg-emerald-400" : status === "connecting" ? "bg-amber-400" : "bg-red-400"}`} />
      {status === "live" ? "Connected" : status === "connecting" ? "Connecting…" : "Disconnected"}
    </p>
    {historyStatus && <p aria-live="polite" className="text-xs text-zinc-400">{historyStatus}</p>}
    {error && <p role="alert" className="text-sm text-amber-200">{error} <button type="button" className="cursor-pointer underline" onClick={() => { setStatus("connecting"); setRetry((value) => value + 1); }}>Retry</button></p>}
    <div ref={host} className="w-full min-w-0 overflow-hidden rounded-xl border border-zinc-800" style={{ height: "max(480px, calc(100dvh - 280px))" }} aria-label={`${selection.coin} ${selection.interval} candlestick and volume chart`} />
  </>;
}
