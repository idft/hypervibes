import { describe, expect, it } from "vitest";
import { randomUUID } from "node:crypto";
import { chartIndicatorEvidence, chartIndicatorInstances } from "./chart-contracts.js";
import { IndicatorError } from "./validation.js";
import type { ChartIndicatorInstance } from "./contracts.js";

const uuid = () => randomUUID();
const instance = (overrides: Partial<ChartIndicatorInstance> = {}): ChartIndicatorInstance => ({ id: uuid(), indicatorId: "builtin:ema", inputs: { period: 20 }, configuredVersion: 1, hidden: false, pane: "auto", styles: {}, ...overrides });
describe("chart indicator configuration", () => {
  it("accepts built-in and same-Agent custom identities with bounded overrides", () => {
    expect(chartIndicatorInstances([instance(), instance({ indicatorId: `custom:${uuid()}` })])).toHaveLength(2);
    expect(chartIndicatorInstances([])).toEqual([]);
    expect(chartIndicatorInstances(undefined)).toEqual([]);
  });
  it("rejects foreign/invalid identities, duplicate instance IDs and unsafe keys", () => {
    for (const value of [[{ ...instance(), indicatorId: "builtin:nope" }], [{ ...instance(), indicatorId: "ema" }], [{ ...instance(), indicatorId: `custom:${uuid()}-extra` }], [{ ...instance(), indicatorId: `custom:${uuid().slice(0, 8)}` }], (() => { const duplicate = instance(); return [duplicate, { ...duplicate }]; })(), [{ ...instance(), inputs: { period: Infinity } }], [{ ...instance(), inputs: JSON.parse('{"__proto__":{"x":1}}') }], [{ ...instance(), pane: "floating" }], [{ ...instance(), hidden: "yes" }], [{ ...instance(), styles: { EMA: { width: 99 } } }], [{ ...instance(), styles: { EMA: { bogus: 1 } } }], Array.from({ length: 11 }, instance)]) {
      expect(() => chartIndicatorInstances(value as unknown[])).toThrow(IndicatorError);
    }
  });
  it("keeps overrides empty by default and preserves explicit values", () => {
    expect(instance().inputs).toEqual({ period: 20 });
    expect(instance({ inputs: {} }).inputs).toEqual({});
    expect(instance({ pane: "separate", hidden: true, styles: { EMA: { color: "#ff9900", width: 2, hidden: true } } })).toMatchObject({ pane: "separate", hidden: true, styles: { EMA: { color: "#ff9900", width: 2, hidden: true } } });
  });
});
const result = { version: 1, sourceHash: "a".repeat(64), engine: { name: "pinets", version: "0.12.1" }, wrapperVersion: 1, resultHash: "b".repeat(64), resolvedInputs: { period: 20 }, inputSchemaHash: "c".repeat(64), computedAt: 0, end: 3600000, dataset: { count: 2, firstOpenTime: 0, lastOpenTime: 3600 }, coverage: { complete: true, closedOnly: true, expected: 2, returned: 2, seedCoverage: "complete" }, sample: [{ time: 0, value: { EMA: 1 } }, { time: 3600, value: { EMA: 2 } }], objects: [{ id: "pine:i:EMA", kind: "shape", time: 3600, text: "UP" }], sampleScope: "partial-visible" };
describe("frozen chart indicator evidence", () => {
  const evidence = (overrides: Record<string, unknown> = {}) => { const configured = instance(); const status = String(overrides.status ?? "current"); return chartIndicatorEvidence([{ instanceId: configured.id, indicatorId: "builtin:ema", configuration: configured, status, ...(status === "loading" ? {} : { result }), ...overrides }]); };
  it("captures identity, provenance, bounded samples and read-only objects", () => {
    expect(evidence()).toHaveLength(1);
    expect(evidence()[0]!.result!.coverage).toMatchObject({ closedOnly: true, seedCoverage: "complete" });
    expect(evidence()[0]!.result!.objects).toEqual([{ id: "pine:i:EMA", kind: "shape", time: 3600, text: "UP" }]);
    expect(evidence({ status: "loading" })[0]!.result).toBeUndefined();
    expect(evidence({ status: "error", error: "Indicator calculation failed" })[0]!.error).toBe("Indicator calculation failed");
  });
  it("rejects mismatched identity, missing provenance and oversized samples", () => {
    for (const value of [{ instanceId: uuid() }, { status: "unknown" }, { result: { ...result, closedOnly: false } }, { result: { ...result, coverage: { complete: true, closedOnly: true, expected: 2, returned: 2, seedCoverage: "later" } } }, { result: { ...result, sample: Array.from({ length: 21 }, (_, i) => ({ time: i, value: { EMA: i } })) } }, { result: { ...result, objects: Array.from({ length: 21 }, (_, i) => ({ id: `x${i}`, kind: "box", time: 0 })) } }, { result: { ...result, sourceHash: "not-a-hash" } }, { result: { ...result, sample: [{ time: 0, value: { EMA: null } }], sampleScope: "complete" } }]) {
      expect(() => { const configured = instance(); chartIndicatorEvidence([{ instanceId: configured.id, indicatorId: "builtin:ema", configuration: configured, ...value }]); }).toThrow(IndicatorError);
    }
    // A pending or stale render is honest evidence; only a missing identity is not.
    for (const status of ["loading", "stale", "error"]) { const configured = instance(); expect(chartIndicatorEvidence([{ instanceId: configured.id, indicatorId: "builtin:ema", configuration: configured, status }])[0]!.status).toBe(status); }
    expect(() => chartIndicatorEvidence(Array.from({ length: 11 }, () => { const configured = instance(); return { instanceId: configured.id, indicatorId: "builtin:ema", configuration: configured, status: "current" }; }))).toThrow(IndicatorError);
  });
});
