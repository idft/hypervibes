import { useEffect, useRef, useState } from "react";
import type { ChartCandle } from "../hyperliquid/chart-contracts.js";
import type { IndicatorResult } from "../indicators/contracts.js";
/** No chart engine import or canvas initialization during SSR. Same renderer as the workspace. */
export function IndicatorPreviewChart({ result, candles }: { result: IndicatorResult; candles: ChartCandle[] }) {
  const host = useRef<HTMLDivElement>(null), [error, setError] = useState("");
  useEffect(() => {
    let disposed = false, close: (() => void) | undefined;
    void Promise.all([import("lightweight-charts"), import("../charts/indicator-renderer.js")]).then(([lib, { createIndicatorRenderer }]) => {
      if (disposed || !host.current) return;
      const chart = lib.createChart(host.current, { autoSize: true, layout: { background: { type: lib.ColorType.Solid, color: "#0d0d10" }, textColor: "#a1a1aa" }, timeScale: { timeVisible: true } });
      const price = chart.addSeries(lib.CandlestickSeries);
      price.setData(candles.map((c) => ({ ...c, time: c.time as import("lightweight-charts").UTCTimestamp })));
      const renderer = createIndicatorRenderer(chart, price);
      close = () => { renderer.close(); chart.remove(); };
      renderer.apply({ id: "preview", indicatorId: result.indicatorId, inputs: {}, configuredVersion: result.version, hidden: false, pane: "auto", styles: {} }, result, candles);
      chart.timeScale().fitContent();
    }).catch((error: unknown) => { close?.(); close = undefined; if (!disposed) setError(error instanceof Error ? error.message : "Preview chart unavailable"); });
    return () => { disposed = true; close?.(); };
  }, [result, candles]);
  return <><div ref={host} className="h-[480px] w-full" aria-label="Read-only indicator preview chart" />{error && <p role="alert">{error}</p>}<p className="text-xs text-zinc-500">Read-only Pine visuals; marker/label glyphs are approximate. Time, direction, text and numerical values are retained.</p></>;
}
