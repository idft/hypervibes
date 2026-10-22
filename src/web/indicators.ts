import { createServerFn } from "@tanstack/react-start";
import { getRequest, getRequestHeaders } from "@tanstack/react-start/server";
import { agentOperation, agentUser, field, optionalNumber } from "./agents.js";
import { record, revision } from "../charts/contracts.js";
import { requestId, AgentError } from "../agents/agent-input.js";
import { validateInputs, validateSource } from "../indicators/validation.js";
import type { IndicatorPreviewRequest, IndicatorSaveRequest } from "../indicators/contracts.js";

function windowInput(value: unknown) {
  const v = record(value), start = optionalNumber(v, "start"), end = optionalNumber(v, "end");
  if (start === undefined || end === undefined) throw new AgentError("An explicit validation start/end is required");
  if (end <= start) throw new AgentError("The validation window must end after it starts");
  return { coin: field(v, "coin"), interval: field(v, "interval"), start, end };
}
export const listIndicators = createServerFn({ method: "GET" }).validator((data: unknown) => {
  const v = record(data), kind = v.kind;
  if (kind !== undefined && kind !== "all" && kind !== "builtin" && kind !== "custom") throw new AgentError("Invalid indicator kind");
  return { agentId: field(v, "agentId"), query: v.query === undefined ? undefined : field(v, "query"), kind: kind as "all" | "builtin" | "custom" | undefined, offset: optionalNumber(v, "offset"), limit: optionalNumber(v, "limit") };
}).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentOperation(async () => { await context.runtime.agents.owned(user.id, data.agentId); return context.runtime.agents.indicators.list(data.agentId, data); });
});
export const getIndicator = createServerFn({ method: "GET" }).validator((data: unknown) => {
  const v = record(data);
  if (v.includeSource !== undefined && typeof v.includeSource !== "boolean") throw new AgentError("Invalid source selection");
  return { agentId: field(v, "agentId"), indicatorId: field(v, "indicatorId"), version: optionalNumber(v, "version"), includeSource: v.includeSource as boolean | undefined, sourceOffset: optionalNumber(v, "sourceOffset"), sourceLimit: optionalNumber(v, "sourceLimit") };
}).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentOperation(async () => { await context.runtime.agents.owned(user.id, data.agentId); return context.runtime.agents.indicators.get(data.agentId, data.indicatorId, data); });
});
export const getIndicatorVersions = createServerFn({ method: "GET" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), indicatorId: field(data, "indicatorId"), offset: optionalNumber(data, "offset"), limit: optionalNumber(data, "limit") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders());
  return agentOperation(async () => { await context.runtime.agents.owned(user.id, data.agentId); return context.runtime.agents.indicators.versions(data.agentId, data.indicatorId, data); });
});
export const previewIndicator = createServerFn({ method: "POST" }).validator((data: unknown) => {
  const v = record(data), request = record(v.request);
  const source = field(request, "source"); validateSource(source);
  const parsed: IndicatorPreviewRequest = { source, ...windowInput(request), inputs: validateInputs(request.inputs ?? {}), count: optionalNumber(request, "count"), markerLimit: optionalNumber(request, "markerLimit") };
  return { agentId: field(v, "agentId"), request: parsed };
}).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentOperation(async () => { await context.runtime.agents.owned(user.id, data.agentId); const result = await context.runtime.agents.indicators.preview(data.agentId, data.request, { signal: getRequest().signal }); const history = await context.runtime.hyperliquid.charts.researchHistory(result.coin, result.interval, data.request.start, result.end, 5000, getRequest().signal); return { result, candles: history.candles }; });
});
export const previewSavedIndicator = createServerFn({ method: "POST" }).validator((data: unknown) => {
  const v = record(data), r = record(v.request);
  return { agentId: field(v, "agentId"), request: { indicatorId: field(r, "indicatorId"), version: optionalNumber(r, "version"), ...windowInput(r), inputs: validateInputs(r.inputs ?? {}), count: optionalNumber(r, "count") } };
}).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentOperation(async () => {
    await context.runtime.agents.owned(user.id, data.agentId);
    const result = await context.runtime.agents.indicators.run(data.agentId, data.request, { signal: getRequest().signal });
    const history = await context.runtime.hyperliquid.charts.researchHistory(result.coin, result.interval, data.request.start, result.end, 5000, getRequest().signal);
    return { result, candles: history.candles };
  });
});
export const saveIndicator = createServerFn({ method: "POST" }).validator((data: unknown) => {
  const v = record(data), r = record(v.request);
  const request: IndicatorSaveRequest = { name: field(r, "name"), description: field(r, "description"), interpretation: field(r, "interpretation"), source: field(r, "source"), indicatorId: r.indicatorId === undefined ? undefined : field(r, "indicatorId"), expectedVersion: optionalNumber(r, "expectedVersion"), validation: r.validation === undefined ? undefined : windowInput(r.validation) };
  return { agentId: field(v, "agentId"), operationKey: requestId(field(v, "operationKey")), request };
}).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentOperation(async () => { await context.runtime.agents.owned(user.id, data.agentId); return context.runtime.agents.indicators.save(data.agentId, data.request, `user:${data.operationKey}`, { kind: "user", userId: user.id }, { signal: getRequest().signal }); });
});
export const renderChartIndicators = createServerFn({ method: "POST" }).validator((data: unknown) => ({ agentId: field(data, "agentId"), layoutId: requestId(field(data, "layoutId")), revision: revision(record(data).revision), end: revision(record(data).end), coin: field(data, "coin"), interval: field(data, "interval") })).handler(async ({ context, data }) => {
  const user = await agentUser(context.runtime, getRequestHeaders(), true);
  return agentOperation(async () => {
    await context.runtime.agents.owned(user.id, data.agentId);
    const layout = await context.runtime.agents.charts.get(data.agentId, data.layoutId);
    if (layout.revision !== data.revision || layout.document.coin !== data.coin || layout.document.interval !== data.interval) throw new AgentError("Chart changed before calculation; refresh the layout", 409);
    return context.runtime.agents.indicators.renderChart(data.agentId, layout, data.end, { signal: getRequest().signal });
  });
});
