export interface ChartFill {
  id: string; time: number; side: "Buy" | "Sell"; size: string; price: string;
}
export interface ChartFillCursor { time: number; id: string }
export interface ChartFillRange { from: number; to: number; cursor?: ChartFillCursor }
export interface ChartFillsPage {
  fills: ChartFill[];
  nextCursor: ChartFillCursor | null;
  historyLimited: boolean;
  sync: { updatedAt: number | null; error: string | null } | null;
}
export interface ChartOverlayVisibility { trades: boolean; position: boolean; orders: boolean }
export const defaultChartOverlays: ChartOverlayVisibility = { trades: true, position: true, orders: true };

/** UTC millisecond range, half-open like a candle. Cursor ties use journal identity. */
export function chartFillRange(data: Record<string, unknown>): ChartFillRange {
  const { from, to, cursor } = data;
  if (typeof from !== "number" || !Number.isSafeInteger(from) || from < 0 || typeof to !== "number" || !Number.isSafeInteger(to) || to <= from) throw new Error("Invalid trade history range");
  if (cursor === undefined) return { from, to };
  if (!cursor || typeof cursor !== "object" || !("time" in cursor) || !("id" in cursor)
    || typeof cursor.time !== "number" || !Number.isSafeInteger(cursor.time) || cursor.time < from || cursor.time >= to
    || typeof cursor.id !== "string" || !/^[a-f0-9]{64}$/.test(cursor.id)) throw new Error("Invalid trade history cursor");
  return { from, to, cursor: { time: cursor.time, id: cursor.id } };
}
