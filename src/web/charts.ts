import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { agentAction, agentUser, field } from "./agents.js";
import { HyperliquidError } from "../hyperliquid/errors.js";

export const getChartInstruments = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(async () => {
      await context.runtime.agents.owned(user.id, data.id);
      return context.runtime.hyperliquid.charts.instruments();
    });
  });

export const getChartHistory = createServerFn({ method: "GET" })
  .validator((data: unknown) => {
    const before = typeof data === "object" && data !== null && "before" in data ? data.before : undefined;
    if (before !== undefined && (typeof before !== "number" || !Number.isSafeInteger(before) || before <= 0)) throw new HyperliquidError("Invalid history cursor");
    return { id: field(data, "id"), coin: field(data, "coin"), interval: field(data, "interval"), before };
  })
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(async () => {
      await context.runtime.agents.owned(user.id, data.id);
      const charts = context.runtime.hyperliquid.charts;
      const selection = await charts.selection(data.coin, data.interval);
      return charts.history(selection, data.before);
    });
  });
