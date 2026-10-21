import { createSeriesMarkers, LineStyle, type CreatePriceLineOptions, type IChartApi, type IPriceLine, type ISeriesApi, type SeriesMarker, type UTCTimestamp } from "lightweight-charts";
import type { AgentDashboard, DashboardOrder } from "../hyperliquid/agent-dashboard.js";
import type { ChartCandle } from "../hyperliquid/chart-contracts.js";
import { compareDecimals, formatMoneyText } from "../hyperliquid/decimal.js";
import type { ChartFill, ChartOverlayVisibility } from "./trading-contracts.js";

const buyColor = "#34d399", sellColor = "#f87171";
const displayPrice = (value: string) => { const price = Number(value); return Number.isFinite(price) && price > 0 ? price : null; };
export function accountSectionFresh(snapshot: AgentDashboard | null, section: "perps" | "orders", interrupted: boolean) {
  return !interrupted && snapshot?.live?.status === "connected" && snapshot.live.sections[section].fresh;
}

/** Numbers are used only for chart coordinates; labels retain exchange decimals. */
export function tradeMarkers(fills: ChartFill[], candles: ChartCandle[], step: number): SeriesMarker<UTCTimestamp>[] {
  const bars = new Set(candles.map((candle) => candle.time));
  return fills.flatMap((fill): SeriesMarker<UTCTimestamp>[] => {
    const time = Math.floor(fill.time / 1000 / step) * step, price = displayPrice(fill.price);
    if (!bars.has(time) || price === null) return [];
    const buy = fill.side === "Buy";
    return [{ id: fill.id, time: time as UTCTimestamp, price, position: buy ? "atPriceBottom" : "atPriceTop", shape: buy ? "arrowUp" : "arrowDown", color: buy ? buyColor : sellColor }];
  }).sort((a, b) => a.time - b.time || String(a.id).localeCompare(String(b.id)));
}

export function accountPriceLines(coin: string, snapshot: AgentDashboard | null, visibility: ChartOverlayVisibility, interrupted: boolean): Map<string, CreatePriceLineOptions> {
  const result = new Map<string, CreatePriceLineOptions>();
  const position = snapshot?.positions?.find((p) => p.coin === coin);
  const size = position?.size.replace(/^-/, "");
  const sign = position ? compareDecimals(position.size, "0") : null;
  if (visibility.position && position && sign !== null && sign !== 0) {
    const price = displayPrice(position.entryPrice);
    const stale = !accountSectionFresh(snapshot, "perps", interrupted);
    if (price !== null) result.set("position", { price, color: stale ? "#a1a1aa" : sign > 0 ? buyColor : sellColor, lineWidth: 2, lineStyle: LineStyle.Solid, axisLabelVisible: true,
      title: `${stale ? "STALE · " : ""}${sign > 0 ? "Long" : "Short"} · ${size} ${coin} · P&L ${compareDecimals(position.unrealizedPnl, "0") === 1 ? "+" : ""}${formatMoneyText(position.unrealizedPnl)} USDC` });
  }
  // Attached TP/SL children can appear only under their pending entry. Prefer a
  // top-level live order when the exchange includes the same identity twice.
  const orders = new Map((snapshot?.orders ?? []).map((order) => [order.id, { order, pending: false }]));
  const children = (order: DashboardOrder) => {
    for (const child of order.children ?? []) {
      if (!orders.has(child.id)) orders.set(child.id, { order: child, pending: true });
      children(child);
    }
  };
  if (visibility.orders) for (const order of snapshot?.orders ?? []) children(order);
  if (visibility.orders) for (const { order, pending } of orders.values()) {
    if (order.coin !== coin) continue;
    const price = displayPrice(order.triggerPrice ?? order.price);
    if (price === null) continue;
    const stale = !accountSectionFresh(snapshot, "orders", interrupted);
    // Position-wide TP/SL orders can report sz=0; do not present that as zero protection.
    const quantity = order.positionTpsl ? "Entire position" : `${order.size} ${coin}`;
    result.set(`order:${order.id}`, { price, color: stale ? "#a1a1aa" : order.side === "Buy" ? buyColor : sellColor, lineWidth: 1,
      lineStyle: order.triggerPrice === undefined ? LineStyle.Dashed : LineStyle.Dotted, axisLabelVisible: true,
      title: `${stale ? "STALE · " : ""}${pending ? "Pending · " : ""}${order.side} ${order.type}${order.triggerPrice === undefined ? "" : " trigger"} · ${quantity}` });
  }
  return result;
}

export interface FillHover { fill: ChartFill; x: number; y: number }
/** Owned by the chart lifecycle: detach primitives before removing the chart. */
export function createTradingOverlays(chart: IChartApi, price: ISeriesApi<"Candlestick">, hover: (value: FillHover | null) => void) {
  const plugin = createSeriesMarkers(price, [], { autoScale: false });
  const lines = new Map<string, IPriceLine>();
  let fillsById = new Map<string, ChartFill>();
  const crosshair: Parameters<IChartApi["subscribeCrosshairMove"]>[0] = (event) => {
    const id = event.hoveredInfo?.objectId;
    const fill = typeof id === "string" ? fillsById.get(id) : undefined;
    hover(fill && event.point ? { fill, x: event.point.x, y: event.point.y } : null);
  };
  chart.subscribeCrosshairMove(crosshair);
  return {
    trades(fills: ChartFill[], candles: ChartCandle[], step: number) {
      fillsById = new Map(fills.map((fill) => [fill.id, fill]));
      plugin.setMarkers(tradeMarkers(fills, candles, step));
      hover(null);
    },
    account(coin: string, snapshot: AgentDashboard | null, visibility: ChartOverlayVisibility, interrupted: boolean) {
      const next = accountPriceLines(coin, snapshot, visibility, interrupted);
      for (const [id, line] of lines) if (!next.has(id)) { price.removePriceLine(line); lines.delete(id); }
      for (const [id, options] of next) {
        const line = lines.get(id);
        if (line) line.applyOptions(options);
        else lines.set(id, price.createPriceLine(options));
      }
    },
    close() {
      chart.unsubscribeCrosshairMove(crosshair);
      plugin.detach();
      for (const line of lines.values()) price.removePriceLine(line);
      lines.clear();
    },
  };
}
