import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { agentAction, agentOperation, agentUser, field } from "./agents.js";
import { chartFillRange } from "../charts/trading-contracts.js";
import { HyperliquidError } from "../hyperliquid/errors.js";
import { requestId } from "../agents/agent-input.js";
import { record, revision } from "../charts/contracts.js";

export const getChartInstruments = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(async () => {
      await context.runtime.agents.owned(user.id, data.id);
      return context.runtime.hyperliquid.charts.instruments();
    });
  });

export const getChartLayouts = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id") }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentAction(async () => {
      await context.runtime.agents.owned(user.id, data.id);
      return context.runtime.agents.charts.list(data.id);
    });
  });

export const createChartLayout = createServerFn({ method: "POST" })
  .validator((data: unknown) => { const row = record(data); return { id: field(data, "id"), layoutId: requestId(field(data, "layoutId")), settings: row.settings, sourceId: row.sourceId === undefined ? undefined : requestId(field(data, "sourceId")) }; })
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(async () => {
      await context.runtime.agents.owned(user.id, data.id);
      return context.runtime.agents.charts.create(data.id, data.layoutId, data.settings, data.sourceId);
    });
  });

export const changeChartLayout = createServerFn({ method: "POST" })
  .validator((data: unknown) => { const row = record(data); return { id: field(data, "id"), layoutId: requestId(field(data, "layoutId")), revision: revision(row.revision), requestId: requestId(field(data, "requestId")), action: field(data, "action"), edit: row.edit }; })
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(async () => {
      await context.runtime.agents.owned(user.id, data.id);
      const charts = context.runtime.agents.charts;
      if (data.action === "edit") return charts.edit(data.id, data.layoutId, data.revision, data.edit, `user:${data.requestId}`);
      if (data.action === "undo" || data.action === "redo") return charts.undo(data.id, data.layoutId, data.revision, `user:${data.requestId}`, data.action === "redo");
      throw new HyperliquidError("Invalid chart action");
    });
  });

export const deleteChartLayout = createServerFn({ method: "POST" })
  .validator((data: unknown) => ({ id: field(data, "id"), layoutId: requestId(field(data, "layoutId")), revision: revision(record(data).revision) }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders(), true);
    return agentAction(async () => {
      await context.runtime.agents.owned(user.id, data.id);
      await context.runtime.agents.charts.delete(data.id, data.layoutId, data.revision);
    });
  });

export const getChartFills = createServerFn({ method: "GET" })
  .validator((data: unknown) => ({ id: field(data, "id"), coin: field(data, "coin"), ...chartFillRange(record(data)) }))
  .handler(async ({ context, data }) => {
    const user = await agentUser(context.runtime, getRequestHeaders());
    return agentOperation(() => context.runtime.hyperliquid.chartFills(user.id, data.id, data.coin, data));
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
