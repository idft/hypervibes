import { infiniteQueryOptions, queryOptions } from "@tanstack/react-query";
import type { ChartFillCursor } from "../charts/trading-contracts.js";
import { getChartFills, getChartHistory, getChartInstruments, getChartLayouts } from "./charts.js";

export const chartInstrumentsQueryOptions = (id: string) => queryOptions({
  queryKey: ["chart-instruments", id],
  queryFn: () => getChartInstruments({ data: { id } }),
});
export const chartLayoutsQueryOptions = (id: string) => queryOptions({
  queryKey: ["chart-layouts", id],
  queryFn: () => getChartLayouts({ data: { id } }),
  // A fresh loader seed precedes the stream. During the mount, only the
  // revision-aware hook may update membership; automatic GETs cannot replace it.
  staleTime: Infinity,
  refetchOnMount: false,
  refetchOnReconnect: false,
});
export const chartHistoryQueryOptions = (id: string, coin: string, interval: string, before?: number) => queryOptions({
  queryKey: ["chart-history", id, coin, interval, before ?? null],
  queryFn: ({ signal }) => getChartHistory({ data: { id, coin, interval, ...(before === undefined ? {} : { before }) }, signal }),
  staleTime: before === undefined ? 0 : 30_000,
  gcTime: 60_000,
});
export const chartFillsQueryOptions = (id: string, coin: string, from: number, to: number) => infiniteQueryOptions({
  queryKey: ["chart-fills", id, coin, from, to],
  queryFn: ({ pageParam, signal }) => getChartFills({ data: { id, coin, from, to, ...(pageParam ? { cursor: pageParam } : {}) }, signal }),
  initialPageParam: undefined as ChartFillCursor | undefined,
  getNextPageParam: (page) => page.nextCursor ?? undefined,
  staleTime: 5_000,
  gcTime: 60_000,
  refetchInterval: 5_000,
  retry: 1,
});
