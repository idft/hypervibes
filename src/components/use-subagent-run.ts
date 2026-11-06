import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { subagentRunQueryOptions } from "../web/subagent-queries.js";
import type { RunDetail } from "../agents/subagent-service.js";

/** One page-scoped subscription; reconnects receive a fresh backend snapshot. */
export function useSubagentRun(agentId: string, subagentId: string, runId: string, initial: RunDetail) {
  const client = useQueryClient();
  const [detail, setDetail] = useState(initial);
  const [connectionError, setConnectionError] = useState("");
  useEffect(() => {
    let active = true;
    setConnectionError("");
    const source = new EventSource(`/api/agents/${encodeURIComponent(agentId)}/sub-agents/${encodeURIComponent(subagentId)}/runs/${encodeURIComponent(runId)}/events`);
    const interrupted = (event: MessageEvent | undefined, fallback: string) => {
      if (!active) return;
      setConnectionError(event ? (JSON.parse(event.data) as { message?: string }).message ?? fallback : fallback);
    };
    source.addEventListener("run", (event) => {
      if (!active) return;
      const next = JSON.parse((event as MessageEvent).data) as RunDetail;
      client.setQueryData(subagentRunQueryOptions(agentId, subagentId, runId).queryKey, next);
      setDetail(next);
      setConnectionError("");
    });
    source.addEventListener("unauthorized", () => { if (active) window.location.assign("/login"); });
    source.addEventListener("shutdown", () => interrupted(undefined, "Server is restarting. Reconnecting…"));
    source.addEventListener("unavailable", (event) => interrupted(event as MessageEvent, "Live updates interrupted. Reconnecting…"));
    source.addEventListener("stream-error", (event) => interrupted(event as MessageEvent, "Run detail could not be streamed."));
    source.onerror = () => interrupted(undefined, "Live updates interrupted. Showing the last received detail. Reconnecting…");
    return () => { active = false; source.close(); };
  }, [agentId, subagentId, runId, client]);
  return { detail, connectionError };
}