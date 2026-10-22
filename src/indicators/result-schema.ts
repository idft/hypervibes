import { Type, validateToolArguments, type JsonObject } from "@earendil-works/pi-ai";
import { indicatorLimits, type IndicatorLimits, type NormalizedIndicator } from "./contracts.js";
import { resolveInputs } from "./validation.js";
import { ArtifactMetaSchema } from "../research/contracts.js";

const object = { additionalProperties: false } as const;
const str = Type.String();
const num = Type.Number();
const optStr = Type.Optional(str);
const optNum = Type.Optional(num);
const scalar = Type.Union([num, str, Type.Boolean()]);
const nullable = Type.Union([num, Type.Null()]);
const pane = { overlay: Type.Boolean(), forceOverlay: Type.Boolean() };
const anchor = Type.Object({ time: num, price: num, barIndex: Type.Optional(Type.Integer()) }, object);
const style = Type.Object({ color: optStr, width: optNum, lineStyle: optStr, hidden: Type.Boolean(), offset: Type.Integer(), showLast: Type.Optional(Type.Integer({ minimum: 0 })) }, object);
const drawing = { id: str, ...pane, color: optStr, width: optNum, lineStyle: optStr, extend: Type.Union([Type.Literal("none"), Type.Literal("left"), Type.Literal("right"), Type.Literal("both")]) };
export const NormalizedIndicatorSchema = Type.Object({
  title: str, overlay: Type.Boolean(),
  inputs: Type.Array(Type.Object({ key: str, runtimeId: str, variable: optStr, title: str,
    type: Type.Union(["int", "float", "bool", "string", "source", "color"].map((v) => Type.Literal(v))),
    default: scalar, min: optNum, max: optNum, step: optNum, options: Type.Optional(Type.Array(scalar)), group: optStr, tooltip: optStr }, object), { maxItems: indicatorLimits.inputs }),
  resolvedInputs: Type.Record(Type.String(), scalar),
  outputs: Type.Array(Type.Object({ id: str, title: str, kind: Type.Union([Type.Literal("line"), Type.Literal("histogram"), Type.Literal("columns")]), ...pane, style,
    finiteCount: Type.Integer({ minimum: 0 }), firstFiniteTime: nullable,
    points: Type.Array(Type.Object({ time: num, displayTime: num, value: nullable, color: optStr }, object), { maxItems: indicatorLimits.candles }) }, object), { maxItems: indicatorLimits.numericOutputs }),
  levels: Type.Array(Type.Object({ id: str, title: str, value: num, ...pane, style }, object), { maxItems: indicatorLimits.numericOutputs }),
  fills: Type.Array(Type.Object({ id: str, title: str, from: str, to: str, color: optStr, hidden: Type.Boolean(), points: Type.Array(Type.Object({ time: num, color: optStr }, object), { maxItems: indicatorLimits.candles }) }, object)),
  markers: Type.Array(Type.Object({ id: str, outputId: str, kind: Type.Union([Type.Literal("shape"), Type.Literal("char"), Type.Literal("arrow")]), time: num, displayTime: num, value: Type.Union([num, Type.Boolean()]), offset: Type.Integer(), ...pane,
    location: str, shape: optStr, text: optStr, character: optStr, color: optStr, textColor: optStr, size: optStr }, object), { maxItems: indicatorLimits.markerEvents }),
  drawings: Type.Array(Type.Union([
    Type.Object({ ...drawing, kind: Type.Literal("line"), from: anchor, to: anchor }, object),
    Type.Object({ ...drawing, kind: Type.Literal("box"), from: anchor, to: anchor, background: optStr, text: str, textColor: optStr, size: optStr }, object),
    Type.Object({ ...drawing, kind: Type.Literal("label"), at: anchor, text: str, textColor: optStr, size: optStr, shape: optStr, location: str }, object),
  ]), { maxItems: indicatorLimits.drawings }),
  values: Type.Array(Type.Object({ time: num, value: Type.Record(Type.String(), nullable) }, object), { maxItems: indicatorLimits.candles }),
  warnings: Type.Array(str, { maxItems: indicatorLimits.warnings }), warningsOmitted: Type.Integer({ minimum: 0 }),
}, object);

const bound = Type.Object({ count: Type.Integer(), firstOpenTime: num, lastOpenTime: num }, object);
const outputBound = Type.Object({ count: Type.Integer(), firstTime: nullable, lastTime: nullable }, object);
const coverage = Type.Object({ complete: Type.Boolean(), closedOnly: Type.Literal(true), expected: Type.Integer(), returned: Type.Integer(), seedCoverage: Type.Union([Type.Literal("complete"), Type.Literal("limited")]) }, object);
const provenance = { ...Type.Omit(NormalizedIndicatorSchema, ["outputs", "levels", "fills", "drawings", "values", "markers"]).properties,
  schemaVersion: Type.Literal(1), indicatorId: str, builtIn: Type.Boolean(), version: Type.Integer(), sourceHash: str,
  engine: Type.Object({ name: Type.Literal("pinets"), version: Type.String({ pattern: "^\\d+\\.\\d+\\.\\d+" }) }, object), wrapperVersion: Type.Integer(), coin: str, interval: str, start: num, end: num,
  inputSchemaHash: str, computedAt: num, candleHash: str, resultHash: str, dataset: bound, outputDataset: outputBound, coverage };
export const IndicatorToolResultSchema = Type.Union([
  Type.Object({ ...provenance, outputs: Type.Array(Type.Omit(NormalizedIndicatorSchema.properties.outputs.items, ["points"])), levels: NormalizedIndicatorSchema.properties.levels, fillCount: Type.Integer(),
    values: NormalizedIndicatorSchema.properties.values, numericalCoverage: Type.Object({ available: Type.Integer(), returned: Type.Integer() }, object),
    markers: NormalizedIndicatorSchema.properties.markers, markerCoverage: Type.Object({ available: Type.Integer(), returned: Type.Integer(), firstSourceTime: nullable, lastSourceTime: nullable }, object),
    drawings: Type.Object({ count: Type.Integer(), byKind: Type.Object({ line: Type.Integer(), box: Type.Integer(), label: Type.Integer() }, object) }, object) }, object),
  Type.Object({ artifact: ArtifactMetaSchema, reference: str, summary: Type.Object({ ...provenance, exportScope: Type.Literal("full") }, object) }, object),
]);

/** Revalidate untrusted VM transport, including finite numbers (JSON must not hide NaN/infinity). */
export function validateNormalizedIndicatorResult(value: unknown, limits: IndicatorLimits = indicatorLimits): NormalizedIndicator {
  const visit = (item: unknown) => {
    if (typeof item === "number" && !Number.isFinite(item)) throw new Error("Non-finite normalized result");
    if (item && typeof item === "object") for (const [key, child] of Object.entries(item)) {
      if (["__proto__", "constructor", "prototype"].includes(key)) throw new Error("Unsafe normalized result key");
      visit(child);
    }
  };
  visit(value);
  const json = JSON.stringify(value);
  if (!json || Buffer.byteLength(json) > limits.resultBytes) throw new Error("Normalized indicator result exceeds byte limit");
  const tool = { name: "indicator_result", description: "Validate VM result", parameters: NormalizedIndicatorSchema };
  const result = validateToolArguments(tool, { type: "toolCall", id: "result", name: tool.name, arguments: value as JsonObject }) as NormalizedIndicator;
  if (result.inputs.length > limits.inputs || result.outputs.length + result.levels.length > limits.numericOutputs || result.markers.length > limits.markerEvents || result.drawings.length > limits.drawings || result.values.length > limits.candles || result.warnings.length > limits.warnings) throw new Error("Normalized indicator result exceeds limits");
  const ids = [...result.outputs, ...result.levels].map((output) => output.id);
  if (new Set(ids).size !== ids.length || new Set(result.inputs.map((input) => input.key)).size !== result.inputs.length) throw new Error("Duplicate normalized result identity");
  resolveInputs(result.inputs, result.resolvedInputs);
  return result;
}
