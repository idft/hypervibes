import { queryOptions } from "@tanstack/react-query";
import { getRiskSettings } from "./risk.js";

export const riskQueryOptions = (agentId: string) => queryOptions({
  queryKey: ["risk", agentId],
  queryFn: () => getRiskSettings({ data: { agentId } }),
});
