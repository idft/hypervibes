import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { ChartLayout } from "../charts/contracts.js";
import { intervalMillis } from "../hyperliquid/chart-contracts.js";
import { chartIndicatorQueryOptions } from "../web/indicator-queries.js";

export function useChartIndicators(agentId: string, layout: ChartLayout) {
  const width = intervalMillis[layout.document.interval];
  const [clock, setClock] = useState(() => Date.now());
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const next = () => { setClock(Date.now()); timer = setTimeout(next, width - Date.now() % width + 1000); };
    next(); return () => clearTimeout(timer);
  }, [width]);
  const end = Math.floor(clock / width) * width;
  const query = useQuery({ ...chartIndicatorQueryOptions(agentId, layout.id, layout.revision, layout.document.coin, layout.document.interval, end), enabled: layout.document.indicators.some((i) => !i.hidden) });
  const identity = JSON.stringify([layout.id, layout.document.coin, layout.document.interval, layout.document.indicators]);
  const previous = useRef<{ identity: string; data: NonNullable<typeof query.data> } | undefined>(undefined);
  useEffect(() => { if (query.data) previous.current = { identity, data: query.data }; }, [identity, query.data]);
  const data = query.data ?? (previous.current?.identity === identity ? previous.current.data : undefined);
  return { data, status: query.isError ? "error" as const : query.isFetching || data?.end !== end ? data ? "stale" as const : "loading" as const : "current" as const, error: query.error?.message, retry: () => { void query.refetch(); } };
}
