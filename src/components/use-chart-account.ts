import { useEffect, useMemo, useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";
import { accountSectionFresh } from "../charts/trading-overlays.js";
import type { ChartFill, ChartOverlayVisibility } from "../charts/trading-contracts.js";
import { chartFillsQueryOptions } from "../web/chart-queries.js";
import { useAgentEvent, useAgentEventsStatus } from "./agent-events.js";

const noFills: ChartFill[] = [];
export function useChartAccount(id: string, coin: string, from: number, to: number, visibility: ChartOverlayVisibility) {
  const [account, setAccount] = useState<{ id: string; snapshot: AgentDashboard | null; interrupted: boolean }>({ id, snapshot: null, interrupted: false });
  const [retry, setRetry] = useState(0);
  const enabled = visibility.position || visibility.orders;
  const { interrupted, streamErrors, reconnect } = useAgentEventsStatus();
  useAgentEvent("dashboard", (event) => {
    setAccount({ id, snapshot: JSON.parse(event.data) as AgentDashboard, interrupted: false });
  }, enabled);
  useEffect(() => {
    setAccount({ id, snapshot: null, interrupted: false });
  }, [id, enabled, retry]);
  useEffect(() => {
    if (!enabled) return;
    const topicError = streamErrors["dashboard"];
    if (topicError || interrupted) setAccount((current) => (current.interrupted ? current : { ...current, interrupted: true }));
  }, [enabled, interrupted, streamErrors]);
  const trades = useInfiniteQuery({
    ...chartFillsQueryOptions(id, coin, from, to),
    enabled: visibility.trades && to > from,
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
    retry: () => { setRetry((n) => n + 1); reconnect(); if (visibility.trades) void trades.refetch(); },
  };
}
