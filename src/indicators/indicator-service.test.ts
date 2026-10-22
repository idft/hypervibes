import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { openDatabase } from "../db/database.js";
import { migrateApplication } from "../db/migrations.js";
import { SharedDatabase } from "../db/shared-database.js";
import { IndicatorService, projectIndicatorResult } from "./indicator-service.js";
import { IndicatorRuntime } from "./runtime.js";
import type { ChartCandle } from "../hyperliquid/chart-contracts.js";
import type { IndicatorInputs } from "./contracts.js";

const hour = 3600_000, end = 100 * hour;
let directory: string, database: SharedDatabase, service: IndicatorService;
const runtime = new IndicatorRuntime();
const execute = vi.spyOn(runtime, "execute");
const candles: ChartCandle[] = Array.from({ length: 100 }, (_, i) => ({ time: i * 3600, open: 100 + i, close: 101 + i, high: 103 + i, low: 99 + i, volume: 10 }));
const market = { researchHistory: vi.fn(async (coin: string, interval: string, start: number, end: number) => ({ coin, interval, start, end, pageEnd: end, candles: candles.filter((c) => c.time * 1000 >= start && c.time * 1000 + hour <= end), nextStart: null, coverage: { complete: true, returned: 100, expected: 100, sourceLimit: "test", closedOnly: true } })) };
const request = { indicatorId: "builtin:ema", coin: "BTC", interval: "1h", start: 0, end, count: 1 };
const source = '//@version=6\nindicator("Agent test")\nperiod = input.int(3, "Period", minval=2)\nplot(ta.sma(close, period), "Average")';
beforeAll(async () => {
  directory = mkdtempSync(join(tmpdir(), "hv-indicator-service-"));
  const connection = openDatabase(join(directory, "test.sqlite")); migrateApplication(connection); database = new SharedDatabase(connection);
  await database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
  for (const id of ["a", "b"]) await database.run("INSERT INTO hv_agents(id,user_id,name,creation_request_id,created_at,updated_at) VALUES (?,'owner',?,?,0,0)", id, id, id);
  service = new IndicatorService(database, market, runtime);
  await service.initialize();
}, 60_000);
afterAll(async () => { await service.close(); await database.close(); rmSync(directory, { recursive: true, force: true }); });

describe("unified PineTS catalog and computation service", () => {
  it("lists compact paginated headers and pages source only when requested", async () => {
    const page = await service.list("a", { limit: 2 });
    expect(page.indicators).toHaveLength(2); expect(page.total).toBe(7); expect(page.nextOffset).toBe(2);
    expect(page.indicators[0]).not.toHaveProperty("inputs"); expect(page.indicators[0]).not.toHaveProperty("source");
    expect(await service.get("a", "builtin:ema")).not.toHaveProperty("source");
    const detail = await service.get("a", "builtin:ema", { includeSource: true, sourceLimit: 10 });
    expect(detail.source!.text).toBe("//@version"); expect(detail.source!.nextOffset).toBe(10);
    expect(await service.versions("a", "builtin:ema")).toMatchObject({ versions: [{ version: 1 }], nextOffset: null });
    await expect(service.get("a", "builtin:ema", { version: 2 })).rejects.toMatchObject({ status: 404 });
  });
  it.each<{ indicatorId: string; inputs: IndicatorInputs; message: string }>([
    { indicatorId: "builtin:bollinger", inputs: { period: 20, mult: 2 }, message: 'Unknown indicator input: "mult". Valid input keys: "period", "deviations", "source".' },
    { indicatorId: "builtin:macd", inputs: { fast: 12, slow: 26, signal: 9 }, message: 'Unknown indicator inputs: "fast", "slow", "signal". Valid input keys: "fastPeriod", "slowPeriod", "signalPeriod", "source", "average", "signalAverage".' },
  ])("rejects guessed $indicatorId keys before fetching candles or launching a VM", async ({ indicatorId, inputs, message }) => {
    const before = execute.mock.calls.length, fetches = market.researchHistory.mock.calls.length;
    const result = service.run("a", { ...request, indicatorId, inputs });
    await expect(result).rejects.toMatchObject({ status: 400, code: "indicator_invalid", message: expect.stringContaining(message) });
    await expect(result).rejects.toThrow("indicator_get");
    expect(execute.mock.calls.length).toBe(before);
    expect(market.researchHistory.mock.calls.length).toBe(fetches);
  });
  it("reports preview input keys from the Pine source inside the VM", async () => {
    await expect(service.preview("a", { ...request, source, inputs: { length: 5 } })).rejects.toThrow('Unknown indicator input: "length". Valid input keys: "period".');
    const noInputs = '//@version=6\nindicator("No inputs")\nplot(close)';
    await expect(service.preview("a", { ...request, source: noInputs, inputs: { period: 5 } })).rejects.toThrow('Unknown indicator input: "period". Valid input keys: (none).');
  });
  it("caches only completed calculations by input/window/candles/Agent and projects after full seeding", async () => {
    const before = execute.mock.calls.length;
    const first = await service.run("a", request);
    const cached = await service.run("a", { ...request, count: 20 });
    expect(execute.mock.calls.length - before).toBe(1);
    expect(cached.resultHash).toBe(first.resultHash); expect(cached.computedAt).toBe(first.computedAt);
    expect(first.dataset.count).toBe(100); expect(first.coverage.complete).toBe(true);
    expect(projectIndicatorResult(first).values).toHaveLength(1);
    expect(projectIndicatorResult(cached, { count: 20 }).values).toHaveLength(20);
    await service.run("a", { ...request, inputs: { period: 5 } });
    await service.run("b", request);
    await service.run("a", { ...request, start: hour });
    expect(execute.mock.calls.length - before).toBe(4);
    candles[99]!.close += 0.25;
    const revised = await service.run("a", request);
    expect(revised.candleHash).not.toBe(first.candleHash);
    expect(execute.mock.calls.length - before).toBe(5);
  });
  it("previews and saves immediately, preserving old executable versions and metadata events", async () => {
    const events: unknown[] = [];
    const unsubscribe = service.subscribe("a", (event) => events.push(event));
    const preview = await service.preview("a", { ...request, source, inputs: { period: 5 } });
    expect(preview.indicatorId).toBe("preview"); expect(preview.resolvedInputs.period).toBe(5);
    const input = { name: "Worker calculation", description: "Saved by test", interpretation: "Analytical", source, validation: { coin: "BTC", interval: "1h", start: 0, end } };
    const first = await service.save("a", input, "create", { kind: "agent", subagentId: "developer", runId: "test-run" });
    expect(first.version).toBe(1); expect(events).toEqual([first]);
    expect((await service.get("a", first.indicatorId)).descriptor.inputs[0]!.default).toBe(3);
    await expect(service.run("a", { ...request, indicatorId: first.indicatorId, inputs: { length: 5 } })).rejects.toThrow('Unknown indicator input: "length". Valid input keys: "period".');
    expect((await service.list("a", { kind: "custom" })).total).toBe(1);
    expect((await service.list("b", { kind: "custom" })).total).toBe(0);
    await expect(service.run("b", { ...request, indicatorId: first.indicatorId })).rejects.toMatchObject({ status: 404 });
    const second = await service.save("a", { ...input, indicatorId: first.indicatorId, expectedVersion: 1, source: source.replace("input.int(3", "input.int(7") }, "update", { kind: "user", userId: "owner" });
    expect(second.version).toBe(2);
    expect((await service.run("a", { ...request, indicatorId: first.indicatorId })).resolvedInputs.period).toBe(7);
    expect((await service.run("a", { ...request, indicatorId: first.indicatorId, version: 1 })).resolvedInputs.period).toBe(3);
    expect((await service.save("a", input, "create", { kind: "agent" })).version).toBe(1);
    unsubscribe();
  });
  it("rejects oversized requests and incomplete explicit windows before computation", async () => {
    const before = execute.mock.calls.length;
    await expect(service.run("a", { ...request, count: 501 })).rejects.toThrow("saveAs");
    await expect(service.run("a", { ...request, markerLimit: 101 })).rejects.toThrow("markerLimit");
    await expect(service.run("a", { ...request, end: end + hour })).rejects.toThrow("Incomplete");
    expect(execute.mock.calls.length).toBe(before);
  });
  it("rejects all new work after close, even cached reads", async () => {
    const isolated = new IndicatorService(database, market);
    await isolated.close();
    await expect(isolated.initialize()).rejects.toThrow("closed");
    await expect(isolated.list("a")).rejects.toMatchObject({ status: 503 });
  });
});
