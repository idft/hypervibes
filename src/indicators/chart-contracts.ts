import { indicatorLimits, type ChartIndicatorEvidence, type ChartIndicatorInstance } from "./contracts.js";
import { IndicatorError, parseIndicatorId, validateInputs } from "./validation.js";

const uuid = (v: unknown) => { if (typeof v !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(v)) throw new IndicatorError("Invalid chart indicator instance ID"); return v; };
const record = (v: unknown) => { if (!v || typeof v !== "object" || Array.isArray(v)) throw new IndicatorError("Invalid chart indicator object"); return v as Record<string, unknown>; };
export function chartIndicatorInstances(value: unknown): ChartIndicatorInstance[] {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.length > indicatorLimits.chartInstances) throw new IndicatorError("A layout supports at most 10 indicators");
  const entries = value.map((item): ChartIndicatorInstance => {
    const v = record(item), id = uuid(v.id);
    parseIndicatorId(v.indicatorId);
    if (!Number.isSafeInteger(v.configuredVersion) || (v.configuredVersion as number) < 1 || typeof v.hidden !== "boolean" || !["auto", "price", "separate"].includes(String(v.pane))) throw new IndicatorError("Invalid chart indicator settings");
    const styles = Object.fromEntries(Object.entries(record(v.styles)).map(([key, item]) => {
      if (["__proto__", "constructor", "prototype"].includes(key)) throw new IndicatorError("Unsafe output style ID");
      const s = record(item);
      if (Object.keys(s).some((k) => !["hidden", "color", "width"].includes(k)) || (s.hidden !== undefined && typeof s.hidden !== "boolean") || (s.color !== undefined && (typeof s.color !== "string" || !/^#[0-9a-f]{6,8}$/i.test(s.color))) || (s.width !== undefined && (typeof s.width !== "number" || !Number.isFinite(s.width) || s.width < 1 || s.width > 5))) throw new IndicatorError("Invalid indicator output style");
      return [key, s];
    }));
    if (Object.keys(styles).length > 64) throw new IndicatorError("Too many indicator styles");
    return { id, indicatorId: v.indicatorId as string, inputs: validateInputs(v.inputs), configuredVersion: v.configuredVersion as number, hidden: v.hidden, pane: v.pane as ChartIndicatorInstance["pane"], styles };
  });
  if (new Set(entries.map((v) => v.id)).size !== entries.length) throw new IndicatorError("Duplicate chart indicator instance IDs");
  return entries;
}
/** Frozen browser evidence, bounded independently and never treated as verified execution authority. */
export function chartIndicatorEvidence(value: unknown): ChartIndicatorEvidence[] {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.length > 10 || new TextEncoder().encode(JSON.stringify(value)).length > 64 * 1024) throw new IndicatorError("Indicator snapshot exceeds evidence budget; zoom in or disable image");
  return value.map((item): ChartIndicatorEvidence => {
    const v = record(item), configuration = chartIndicatorInstances([v.configuration])[0]!;
    if (v.instanceId !== configuration.id || v.indicatorId !== configuration.indicatorId || !["loading", "current", "stale", "error"].includes(String(v.status)) || (v.error !== undefined && (typeof v.error !== "string" || v.error.length > 1000))) throw new IndicatorError("Invalid indicator evidence identity");
    const base = { instanceId: configuration.id, indicatorId: configuration.indicatorId, configuration, status: v.status as ChartIndicatorEvidence["status"], ...(v.error === undefined ? {} : { error: v.error as string }) };
    if (v.result === undefined) return base;
    const r = record(v.result);
    const finite = (v: unknown): number => { if (typeof v !== "number" || !Number.isFinite(v)) throw new IndicatorError("Invalid indicator evidence number"); return v; };
    const hash = (v: unknown): string => { if (typeof v !== "string" || !/^[0-9a-f]{64}$/.test(v)) throw new IndicatorError("Invalid indicator evidence hash"); return v; };
    if (!Array.isArray(r.sample) || r.sample.length > 20 || !Array.isArray(r.objects) || r.objects.length > 20 || r.sampleScope !== "partial-visible") throw new IndicatorError("Invalid indicator evidence sample");
    const dataset = record(r.dataset), coverage = record(r.coverage), engine = record(r.engine);
    if (engine.name !== "pinets" || typeof engine.version !== "string" || !/^\d+\.\d+\.\d+/.test(engine.version) || coverage.closedOnly !== true || typeof coverage.complete !== "boolean" || !["complete", "limited"].includes(String(coverage.seedCoverage))) throw new IndicatorError("Invalid indicator evidence provenance");
    return { ...base, result: { version: finite(r.version), sourceHash: hash(r.sourceHash), engine: { name: "pinets", version: engine.version }, wrapperVersion: finite(r.wrapperVersion), resultHash: hash(r.resultHash), resolvedInputs: validateInputs(r.resolvedInputs), inputSchemaHash: hash(r.inputSchemaHash), computedAt: finite(r.computedAt), end: finite(r.end),
      dataset: { count: finite(dataset.count), firstOpenTime: finite(dataset.firstOpenTime), lastOpenTime: finite(dataset.lastOpenTime) }, coverage: { complete: coverage.complete, closedOnly: true, expected: finite(coverage.expected), returned: finite(coverage.returned), seedCoverage: coverage.seedCoverage as "complete" | "limited" }, sampleScope: "partial-visible",
      sample: r.sample.map((p) => { const point = record(p); const values = Object.fromEntries(Object.entries(record(point.value)).map(([key, value]) => { if (["__proto__", "constructor", "prototype"].includes(key)) throw new IndicatorError("Invalid evidence output key"); return [key, value === null ? null : finite(value)]; })); if (Object.keys(values).length > 32) throw new IndicatorError("Too many evidence outputs"); return { time: finite(point.time), value: values }; }),
      objects: r.objects.map((object) => { const o = record(object); if (typeof o.id !== "string" || o.id.length > 200 || typeof o.kind !== "string" || o.kind.length > 32 || (o.text !== undefined && (typeof o.text !== "string" || o.text.length > 1000))) throw new IndicatorError("Invalid generated-object evidence"); return { id: o.id, kind: o.kind, time: finite(o.time), ...(o.text === undefined ? {} : { text: o.text as string }) }; }) } };
  });
}
