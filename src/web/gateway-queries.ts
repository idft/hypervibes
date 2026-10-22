import { queryOptions } from "@tanstack/react-query";
import { getGatewayStatus } from "./gateway.js";

export const gatewayQueryOptions = (id: string) => queryOptions({
  queryKey: ["gateway", id],
  queryFn: () => getGatewayStatus({ data: { id } }),
  // The section refetches after each mutation; no other owner exists.
  staleTime: Infinity,
});