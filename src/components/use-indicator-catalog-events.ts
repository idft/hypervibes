import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { indicatorKeys } from "../web/indicator-queries.js";
import { useAgentEvent, useAgentEventsStatus } from "./agent-events.js";
/** Mounted once by AgentLayout, not once per instance/chart. */
export function useIndicatorCatalogEvents(agentId: string) {
  const client = useQueryClient();
  const { interrupted, controlMessage, streamErrors } = useAgentEventsStatus();
  const [error, setError] = useState("");
  useAgentEvent("indicators", () => {
    setError("");
    void client.invalidateQueries({ queryKey: indicatorKeys.all(agentId) });
  });
  useEffect(() => {
    const topicError = streamErrors["indicators"];
    if (topicError) setError("Indicator updates unavailable. Reload to retry (check open-tab connection limits).");
    else if (interrupted) setError(controlMessage || "Indicator updates disconnected. Latest versions may be stale; reconnecting…");
  }, [interrupted, controlMessage, streamErrors]);
  return error;
}
