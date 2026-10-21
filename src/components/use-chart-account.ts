import { useEffect, useMemo, useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";
import { accountSectionFresh } from "../charts/trading-overlays.js";
import type { ChartFill, ChartFillCursor, ChartOverlayVisibility } from "../charts/trading-contracts.js";
import { getChartFills } from "../web/charts.js";

const noFills: ChartFill[] = [];
export function useChartAccount(id: string, coin: string, from: number, to: number, visibility: ChartOverlayVisibility) {
  const [account, setAccount] = useState<{ id: string; snapshot: AgentDashboard | null; interrupted: boolean }>({ id, snapshot: null, interrupted: false });
  const [retry, setRetry] = useState(0);
  const enabled = visibility.position || visibility.orders;
  useEffect(() => {
    setAccount({ id, snapshot: null, interrupted: false });
    if (!enabled) return;
    let disposed = false;
    const source = new EventSource(`/api/agents/${id}/dashboard/events`);
    source.addEventListener("dashboard", (event) => {
      if (!disposed) setAccount({ id, snapshot: JSON.parse((event as MessageEvent).data) as AgentDashboard, interrupted: false });
    });
    const interrupted = () => { if (!disposed) setAccount((current) => ({ ...current, interrupted: true })); };
    source.onerror = interrupted;
    for (const name of ["shutdown", "unavailable"]) source.addEventListener(name, interrupted);
    source.addEventListener("stream-error", () => { source.close(); interrupted(); });
    source.addEventListener("unauthorized", () => { source.close(); window.location.assign("/login"); });
    return () => { disposed = true; source.close(); };
  }, [id, enabled, retry]);
  const trades = useInfiniteQuery({
    queryKey: ["chart-fills", id, coin, from, to],
    queryFn: ({ pageParam, signal }) => getChartFills({ data: { id, coin, from, to, ...(pageParam ? { cursor: pageParam } : {}) }, signal }),
    initialPageParam: undefined as ChartFillCursor | undefined,
    getNextPageParam: (page) => page.nextCursor ?? undefined,
    enabled: visibility.trades && to > from,
    staleTime: 5_000,
    gcTime: 60_000,
    refetchInterval: 5_000,
    retry: 1,
  });
  const { hasNextPage, isFetching, isError, fetchNextPage } = trades;
  useEffect(() => {
    if (visibility.trades && hasNextPage && !isFetching && !isError) void fetchNextPage();
  }, [visibility.trades, hasNextPage, isFetching, isError, fetchNextPage]);
  const fills = useMemo(() => {
    if (!trades.data) return noFills;
    const byId = new Map<string, ChartFill>();
    for (const page of trades.data.pages) for (const fill of page.fills) byId.set(fill.id, fill);
    return [...byId.values()];
  }, [trades.data]);
  const snapshot = account.id === id ? account.snapshot : null;
  const messages: string[] = [];
  if (enabled) {
    if (!snapshot) messages.push(account.interrupted ? "Account unavailable · Retry to reconnect" : "Connecting account…");
    else {
      if (visibility.position && !accountSectionFresh(snapshot, "perps", account.interrupted)) messages.push("Position data stale");
      if (visibility.orders && !accountSectionFresh(snapshot, "orders", account.interrupted)) messages.push("Order data stale");
    }
  }
  if (visibility.trades && to > from) {
    if (isError) messages.push("Trade history unavailable · displayed markers may be stale");
    else if (trades.isPending || hasNextPage) messages.push("Loading trade history…");
    const page = trades.data?.pages[0];
    if (page && (!page.sync?.updatedAt || page.sync.error)) messages.push("Trade journal syncing or unavailable · history may be incomplete");
    if (page?.historyLimited) messages.push("Exchange history is limited · showing available journal fills");
  }
  return { snapshot, interrupted: account.interrupted, fills, messages,
    canRetry: (enabled && account.interrupted) || (visibility.trades && isError),
    retry: () => { setRetry((n) => n + 1); if (visibility.trades) void trades.refetch(); },
  };
}
