import { queryOptions } from "@tanstack/react-query";
import { getAccount } from "./account.js";

export const accountQueryOptions = () => queryOptions({
  queryKey: ["account"],
  queryFn: () => getAccount(),
});
