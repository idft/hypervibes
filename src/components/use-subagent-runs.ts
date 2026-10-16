import { useEffect, useState } from "react";
import { runPagination, type RunHistory, type RunPagination, type RunSummary } from "../agents/subagent-history.js";

type InitialHistory = { runs: RunSummary[]; pagination?: RunPagination; schedulerError: string | null };
/** One page-scoped subscription; reconnects receive a fresh backend snapshot. */
export function useSubagentRuns(agentId: string, subagentId: string | undefined, initial?: InitialHistory) {
  const [history, setHistory] = useState<RunHistory>(() => ({ runs: initial?.runs ?? [], pagination: initial?.pagination ?? runPagination(initial?.runs.length ?? 0), schedulerError: initial?.schedulerError ?? null }));
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(false);
  const [connectionError, setConnectionError] = useState("");
  useEffect(() => {
    if (!subagentId) return;
    let active = true;
    setLoading(true); setConnectionError("");
    const source = new EventSource(`/api/agents/${encodeURIComponent(agentId)}/sub-agents/${encodeURIComponent(subagentId)}/runs/events?page=${page}`);
    const interrupted = (event: MessageEvent | undefined, fallback: string) => {
      if (!active) return;
      setLoading(false);
      setConnectionError(event ? (JSON.parse(event.data) as { message?: string }).message ?? fallback : fallback);
    };
    source.addEventListener("runs", (event) => {
      if (!active) return;
      const snapshot = JSON.parse((event as MessageEvent).data) as RunHistory;
      setHistory(snapshot); setLoading(false); setConnectionError("");
      if (snapshot.pagination.page !== page) setPage(snapshot.pagination.page);
    });
    source.addEventListener("unauthorized", () => { if (active) window.location.assign("/login"); });
    source.addEventListener("shutdown", () => interrupted(undefined, "Server is restarting. Reconnecting…"));
    source.addEventListener("unavailable", (event) => interrupted(event as MessageEvent, "Run history updates interrupted. Reconnecting…"));
    source.addEventListener("stream-error", (event) => interrupted(event as MessageEvent, "Run history could not be streamed."));
    source.onerror = () => interrupted(undefined, "Live updates interrupted. Showing the last received history. Reconnecting…");
    return () => { active = false; source.close(); };
  }, [agentId, subagentId, page]);
  return { history, loading, connectionError, setPage };
}
