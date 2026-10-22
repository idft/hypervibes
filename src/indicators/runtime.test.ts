import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { BuiltinIndicatorCatalog } from "./builtin-catalog.js";
import { IndicatorRuntime, pineVersionDrift } from "./runtime.js";
import { readFile } from "node:fs/promises";
import { validateToolArguments, type JsonObject } from "@earendil-works/pi-ai";
import { projectIndicatorResult } from "./indicator-service.js";
import { IndicatorToolResultSchema } from "./result-schema.js";
import type { IndicatorResult } from "./contracts.js";
import { type IndicatorInputs, type PineCandle } from "./contracts.js";

const candles: PineCandle[] = Array.from({ length: 80 }, (_, i) => {
  const close = 100 + i * 0.3 + Math.sin(i * 1.7) * 4;
  return { open: close + 0.5, high: close + 3, low: close - 3, close, volume: 100 + i % 7, openTime: i * 3600_000, closeTime: (i + 1) * 3600_000 };
});
const runtime = new IndicatorRuntime();
const catalog = new BuiltinIndicatorCatalog();
const run = (source: string, inputs: IndicatorInputs = {}, data = candles, owner = runtime, signal?: AbortSignal) => owner.execute("agent-a", { source, inputs, candles: data, coin: "BTC", pineTimeframe: "60", intervalMs: 3600_000 }, signal);
const program = (body: string, overlay = false) => `//@version=6\nindicator("Test", overlay=${overlay})\n${body}`;
beforeAll(async () => { await runtime.initialize(); await catalog.initialize(runtime); }, 60_000);
afterAll(async () => { await runtime.close(); });

describe("bounded no-capability PineTS runtime", () => {
  it("loads all seven native built-ins and exposes meaningful validated input bindings", async () => {
    expect(catalog.list()).toHaveLength(7);
    for (const descriptor of catalog.list()) {
      const result = await run(catalog.get(descriptor.id).source);
      expect(result.overlay).toBe(descriptor.overlay);
      const latest = result.values.at(-1)!;
      expect(Object.values(latest.value).every((value) => typeof value === "number" && Number.isFinite(value))).toBe(true);
      expect(result.inputs.find((input) => input.key === "period" || input.key === "fastPeriod")).toBeDefined();
      if (descriptor.id === "builtin:rsi") expect(latest.value.RSI).toBeGreaterThanOrEqual(0);
      if (descriptor.id === "builtin:rsi") expect(latest.value.RSI).toBeLessThanOrEqual(100);
      if (descriptor.id === "builtin:adx") expect(latest.value.ADX).toBeGreaterThanOrEqual(0);
      if (descriptor.id === "builtin:bollinger") {
        expect(latest.value.Upper).toBeGreaterThan(latest.value.Middle!);
        expect(latest.value.Lower).toBeLessThan(latest.value.Middle!);
        expect(result.fills).toHaveLength(1);
      }
    }
  }, 60_000);
  it("calculates SMA against independent arithmetic and changes inputs", async () => {
    const source = catalog.get("builtin:sma").source;
    const result = await run(source, { period: 7, source: "close" });
    expect(result.values.at(-1)!.value.SMA).toBeCloseTo(candles.slice(-7).reduce((sum, c) => sum + c.close, 0) / 7, 10);
    expect(result.resolvedInputs).toEqual({ period: 7, source: "close" });
    expect(result.values[0]!.value.SMA).toBeNull();
    expect((await run(source, { period: 3, source: "open" })).values.at(-1)).not.toEqual(result.values.at(-1));
  });
  it("rejects wrong types, unknown keys, out-of-range values and invalid options through validated setter", async () => {
    const source = catalog.get("builtin:rsi").source;
    for (const inputs of [{ period: "14" }, { period: 1 }, { period: 2.5 }, { smoothing: "invalid" }, { absent: 1 }] as IndicatorInputs[]) await expect(run(source, inputs)).rejects.toThrow();
    expect(() => catalog.resolve("builtin:macd", { fastPeriod: 40, slowPeriod: 20 })).toThrow("smaller");
  }, 30_000);
  it("accepts v5, helpers, bounded loops, history and persistent variables", async () => {
    const result = await run(program(`var counter = 0\nf_double(x) =>\n    x * 2\nfor i = 0 to 2\n    counter += 1\nplot(f_double(counter), "Counter")\nplot(close[1], "Previous")`).replace("version=6", "version=5"));
    expect(result.values.at(-1)!.value.Counter).toBe(candles.length * 6);
    expect(result.values.at(-1)!.value.Previous).toBe(candles.at(-2)!.close);
  });
  it("preserves PineTS last closed input bar realtime semantics", async () => {
    const result = await run(program('plot(barstate.isrealtime ? 1 : 0, "Realtime")\nplot(barstate.ishistory ? 1 : 0, "History")\nplot(barstate.isconfirmed ? 1 : 0, "Confirmed")'));
    expect(result.values.at(-1)!.value).toEqual({ Realtime: 1, History: 0, Confirmed: 1 });
  });
  it("normalizes duplicate titles, signed arrows, fills, zero levels and actual drawing anchors", async () => {
    const source = program(`p = plot(close, "Same")\nq = plot(open, "Same", style=plot.style_histogram)\nfill(p, q, color=color.new(color.blue, 90))\nhline(0, "Zero")\nplotarrow(bar_index % 3 - 1, "Arrow", offset=2)\nplotshape(bar_index % 2 == 0, "Signal", text="Known", offset=-1)\nif barstate.islast\n    removed = line.new(0, 0, 1, 1)\n    line.delete(removed)\n    line.new(bar_index-2, -1, bar_index+2, 0, extend=extend.right)\n    box.new(time[2], 3, time, -3, xloc=xloc.bar_time, text="Box")\n    label.new(bar_index, close, "Last")`, true);
    const result = await run(source);
    expect(result.outputs.map((output) => output.id)).toEqual(["Same", "Same#1"]);
    expect(result.levels[0]!.value).toBe(0);
    expect(result.fills[0]).toMatchObject({ from: "Same", to: "Same#1" });
    const arrow = result.markers.find((marker) => marker.kind === "arrow")!;
    expect(arrow.value).toBe(-1); expect(arrow.displayTime - arrow.time).toBe(7200);
    expect(result.drawings).toHaveLength(3);
    expect(result.drawings.find((drawing) => drawing.kind === "line")).toMatchObject({ from: { time: 77 * 3600, price: -1 }, to: { time: 81 * 3600, price: 0 }, extend: "right" });
    expect(result.drawings.find((drawing) => drawing.kind === "box")).toMatchObject({ text: "Box", from: { time: 77 * 3600 }, to: { time: 79 * 3600 } });
  });
  it("rejects nonempty unsupported visuals, but empty reserved collections are normal", async () => {
    await expect(run(program('var t = table.new(position.top_right, 1, 1)\nif barstate.islast\n    table.cell(t, 0, 0, "Unsupported")\nplot(close)'))).rejects.toThrow(/Unsupported nonempty.*__tables__/);
    await expect(run(program('plot(close, style=plot.style_area)'))).rejects.toThrow("Unsupported nonempty");
    await expect(run(program('plot(close)'))).resolves.toHaveProperty("values");
  });
  it("does not expose host capabilities through Pine source injection", async () => {
    const source = program(String.raw`plot(close, "\"; return process.env; //")`);
    const result = await run(source);
    expect(result.outputs[0]!.title).toContain("process.env");
    expect(result.values).toHaveLength(candles.length);
    await expect(run(program('plot(request.security("BTC", "60", close))'))).rejects.toThrow("request.*");
  });
  it("enforces loops, cancellation and recovery", async () => {
    await expect(run(program('while true\n    x = 1\nplot(close)'))).rejects.toThrow(/loop|Loop/);
    const controller = new AbortController();
    const task = run(program('plot(close)'), {}, candles, runtime, controller.signal);
    controller.abort();
    await expect(task).rejects.toThrow();
    await expect.poll(async () => (await run(program('plot(close)'))).values.length).toBe(candles.length);
  });
  it("runs a seven-indicator Promise.all sweep for one Agent", async () => {
    expect(runtime.limits.workers).toBe(8);
    const requests = [
      { name: "ema", inputs: { period: 20 } }, { name: "ema", inputs: { period: 50 } },
      ...["atr", "rsi", "macd", "adx", "bollinger"].map((name) => ({ name, inputs: {} })),
    ];
    const results = await Promise.all(requests.map(({ name, inputs }) => run(catalog.get(`builtin:${name}`).source, inputs)));
    expect(results).toHaveLength(7);
    for (const result of results) expect(result.values).toHaveLength(candles.length);
  }, 60_000);
  it("drains concurrent single and batch calculations through one global worker", async () => {
    const bounded = new IndicatorRuntime({ workers: 1 });
    await bounded.initialize();
    try {
      const source = program('plot(close)');
      const results = await Promise.all([
        run(source, {}, candles, bounded),
        bounded.executeBatch("agent-a", [{ source, inputs: {}, candles, coin: "BTC", pineTimeframe: "60", intervalMs: 3600_000 }]),
        run(source, {}, candles, bounded),
      ]);
      expect(results[0].values).toHaveLength(candles.length);
      expect(results[1][0]!.values).toHaveLength(candles.length);
      expect(results[2].values).toHaveLength(candles.length);
    } finally { await bounded.close(); }
  }, 30_000);
  it("permanently aborts a deleted Agent's active and queued calculations", async () => {
    const bounded = new IndicatorRuntime({ workers: 1 });
    await bounded.initialize();
    try {
      const tasks = Array.from({ length: 3 }, () => expect(run(program('plot(close)'), {}, candles, bounded)).rejects.toThrow());
      await bounded.abortAgent("agent-a");
      await Promise.all(tasks);
      await expect(run(program('plot(close)'), {}, candles, bounded)).rejects.toMatchObject({ status: 503 });
    } finally { await bounded.close(); }
  });
  it("derives the recorded engine version from the installed package and only warns on drift", async () => {
    const pinets = JSON.parse(await readFile(new URL("../package.json", import.meta.resolve("pinets")), "utf8")) as { version: string };
    expect(runtime.engine.version).toBe(pinets.version);
    expect(pineVersionDrift(pinets.version, pinets.version)).toBeNull();
    expect(pineVersionDrift("0.13.0", "0.12.1")).toContain("0.13.0");
    // A drifted engine must not brick the tool result contract: the schema
    // records whatever semver the installed engine reported.
    const result = await run(catalog.get("builtin:ema").source);
    const drifted: IndicatorResult = { ...result, schemaVersion: 1, indicatorId: "builtin:ema", builtIn: true, version: 1, sourceHash: "a".repeat(64), engine: { name: "pinets", version: "0.13.0" }, wrapperVersion: 1, coin: "BTC", interval: "1h", start: 0, end: candles.length * 3600_000, inputSchemaHash: "b".repeat(64), computedAt: Date.now(), candleHash: "c".repeat(64), resultHash: "d".repeat(64), dataset: { count: candles.length, firstOpenTime: 0, lastOpenTime: candles.length - 1 }, outputDataset: { count: candles.length, firstTime: 0, lastTime: candles.length - 1 }, coverage: { complete: true, closedOnly: true, expected: candles.length, returned: candles.length, seedCoverage: "complete" } };
    const projection = projectIndicatorResult(drifted) as unknown as JsonObject;
    expect(() => validateToolArguments({ name: "indicator_run", description: "Validate", parameters: IndicatorToolResultSchema }, { type: "toolCall", id: "drift", name: "indicator_run", arguments: projection })).not.toThrow();
  });
  it("runs under the actual tsx backend loader, including trusted compiler helpers", async () => {
    const payload = { source: program('plot(close, "Close")'), coin: "BTC", pineTimeframe: "60", intervalMs: 3600_000, candles: candles.slice(0, 2), inputs: {} };
    const code = `const timer = setInterval(() => {}, 1000);
      import(${JSON.stringify(new URL("./runtime.ts", import.meta.url).href)}).then(async ({ IndicatorRuntime }) => {
        const runtime = new IndicatorRuntime();
        try {
          await runtime.initialize();
          const result = await runtime.execute("loader-test", ${JSON.stringify(payload)});
          console.log(JSON.stringify(result.values));
          await runtime.execute("loader-test", { ...${JSON.stringify(payload)}, inputs: { typo: 1 } }).catch(error => console.error(error.message));
        } finally { await runtime.close(); }
      }).catch(error => { console.error(error); process.exitCode = 1; }).finally(() => clearInterval(timer));`;
    const result = await promisify(execFile)(process.execPath, ["--import", "tsx", "--eval", code], { timeout: 20_000 });
    expect(JSON.parse(result.stdout)).toHaveLength(2);
    expect(result.stderr).toContain('Unknown indicator input: "typo". Valid input keys: (none).');
  }, 30_000);
  it("enforces the VM deadline and heap budget, then closes actual workers", async () => {
    for (const budgets of [{ timeoutMs: 1 }, { memoryLimitBytes: 16 * 1024 * 1024 }]) {
      const bounded = new IndicatorRuntime(budgets);
      await bounded.initialize();
      const source = "memoryLimitBytes" in budgets ? program('var huge = array.new_string(0)\nfor i = 0 to 9999\n    array.push(huge, str.repeat(str.tostring(i), 10000))\nplot(array.size(huge))') : program('plot(close)');
      try { await expect(run(source, {}, candles, bounded)).rejects.toThrow(/timeout|timed out|memory|allocat/i); }
      finally { await bounded.close(); }
    }
  });
  it("bounds normalized result bytes inside the VM", async () => {
    const bounded = new IndicatorRuntime({ resultBytes: 100 });
    await bounded.initialize();
    try { await expect(run(program('plot(close)'), {}, candles, bounded)).rejects.toThrow("byte limit"); }
    finally { await bounded.close(); }
  });
});
