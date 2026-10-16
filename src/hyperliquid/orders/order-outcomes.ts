import type { OrderStatusResponse } from "@nktkas/hyperliquid";
import type { OrderView } from "./order-contracts.js";
import { liveView } from "./order-prepare.js";
import { compare, decimal } from "./order-math.js";

function record(value: unknown): Record<string, unknown> | undefined { return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined; }
export function unknownViews(views: OrderView[], message: string): OrderView[] {
  return views.map((view) => ({ ...view, commandStatus: "unknown", orderStatus: "unknown", source: "transport", updatedAt: Date.now(), error: message }));
}
/** SDK throws for mixed-success batches; decode its raw response item by item. */
export function responseViews(kind: "submit" | "cancel", response: unknown, views: OrderView[]): OrderView[] {
  const root = record(response);
  if (root?.status === "err" && typeof root.response === "string") {
    if (/nonce|duplicate|already/i.test(root.response)) return unknownViews(views, "Exchange reported a duplicate identity or nonce; reconcile before any new attempt.");
    return views.map((view) => ({ ...view, commandStatus: "rejected", orderStatus: kind === "submit" ? "rejected" : view.orderStatus, error: (root.response as string).slice(0, 1000), updatedAt: Date.now(), source: "exchange_response" }));
  }
  const body = record(root?.response);
  const statuses = record(body?.data)?.statuses;
  if (root?.status !== "ok" || body?.type !== (kind === "submit" ? "order" : "cancel") || !Array.isArray(statuses) || statuses.length !== views.length) return unknownViews(views, "Exchange response shape/count is uncertain. Reconcile; do not resubmit.");
  return views.map((view, index) => {
    const item: unknown = statuses[index];
    const result = { ...view, updatedAt: Date.now(), source: "exchange_response" };
    if (kind === "cancel" && item === "success") return { ...result, commandStatus: "accepted", orderStatus: "canceled", error: null };
    const status = record(item);
    if (typeof status?.error === "string") {
      if (kind === "submit" && /duplicate|cloid/i.test(status.error)) return unknownViews([view], "Duplicate client order ID response; reconcile before retrying.")[0]!;
      return { ...result, commandStatus: "rejected", orderStatus: kind === "submit" ? "rejected" : view.orderStatus, error: status.error.slice(0, 1000) };
    }
    if (kind === "submit") {
      if (item === "waitingForFill" || item === "waitingForTrigger") return { ...result, commandStatus: "accepted", orderStatus: item, error: null };
      const resting = record(status?.resting), filled = record(status?.filled);
      const oid = resting?.oid ?? filled?.oid;
      if (typeof oid === "number" && Number.isSafeInteger(oid) && oid > 0 && (resting || filled)) {
        if (filled) {
          try {
            decimal(filled.totalSz as string); decimal(filled.avgPx as string);
            if (view.size && compare(filled.totalSz as string, view.size) > 0) throw new Error("Overfilled receipt");
          } catch { return unknownViews([view], "Malformed fill receipt; reconciliation required.")[0]!; }
        }
        const responseCloid = resting?.cloid ?? filled?.cloid;
        if (responseCloid && responseCloid !== view.cloid) return unknownViews([view], "Exchange receipt client order ID did not match intent.")[0]!;
        return { ...result, commandStatus: "accepted", orderStatus: filled ? "filled" : "open", exchangeOid: String(oid),
          filledSize: filled ? filled.totalSz as string : null, averageFillPrice: filled ? filled.avgPx as string : null, error: null };
      }
    }
    return unknownViews([view], "Unrecognized per-order outcome; reconciliation required.")[0]!;
  });
}

/** A targeted not-found result never manufactures a rejection or cancellation. */
export function reconciledView(view: OrderView, response: OrderStatusResponse): OrderView {
  if (response.status !== "order") return { ...view, updatedAt: Date.now(), error: "Order not found in venue lookup; absence is not proof of rejection, fill, or cancellation." };
  const report = response.order;
  const remote = liveView(report.order, report.status, report.statusTimestamp);
  if (remote.symbol !== view.symbol || (view.exchangeOid && remote.exchangeOid !== view.exchangeOid) || (view.cloid && remote.cloid && view.cloid !== remote.cloid)) {
    return { ...view, updatedAt: Date.now(), error: "Exchange order identity did not match the recorded intent." };
  }
  if (view.venueTimestamp !== null && report.statusTimestamp < view.venueTimestamp) return view;
  // A pending cancel is not confirmed by seeing an order still open/triggered.
  const terminal = !["open", "triggered"].includes(report.status);
  return { ...view, exchangeOid: remote.exchangeOid, orderStatus: report.status, venueTimestamp: report.statusTimestamp,
    commandStatus: view.kind === "cancel" && !terminal && view.commandStatus === "unknown" ? "unknown"
      : view.commandStatus === "rejected" ? "rejected" : "reconciled",
    updatedAt: Date.now(), source: "reconciliation", error: null };
}
