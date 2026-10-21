import { queryOptions } from "@tanstack/react-query";
import { readArtifact } from "./artifacts.js";

export const artifactQueryOptions = (agentId: string, id: string, offset = 0) => queryOptions({
  queryKey: ["artifacts", agentId, id, offset],
  queryFn: () => readArtifact({ data: { agentId, id, offset } }),
  // Recheck retention and memory protection whenever a preview is opened.
  staleTime: 0,
});
