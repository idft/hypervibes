import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { indicatorKeys } from "../web/indicator-queries.js";
/** Mounted once by AgentLayout, not once per instance/chart. */
export function useIndicatorCatalogEvents(agentId: string) {
  const client = useQueryClient();
  const [error, setError] = useState("");
  useEffect(() => {
    const events = new EventSource(`/api/agents/${agentId}/indicators/events`);
    events.addEventListener("indicators", () => { setError(""); void client.invalidateQueries({ queryKey: indicatorKeys.all(agentId) }); });
    const interrupted = () => setError("Indicator updates disconnected. Latest versions may be stale; reconnecting…");
    events.onerror = interrupted;
    events.addEventListener("shutdown", interrupted);
    events.addEventListener("stream-error", () => { events.close(); setError("Indicator updates unavailable. Reload to retry (check open-tab connection limits)."); });
    events.addEventListener("unauthorized", () => { events.close(); window.location.assign("/login"); });
    events.addEventListener("unavailable", () => { events.close(); interrupted(); });
    return () => events.close();
  }, [client, agentId]);
  return error;
}
