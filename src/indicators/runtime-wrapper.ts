import type * as PineTS from "pinets";
import type { IndicatorInput, IndicatorInputValue, IndicatorVmRequest, NormalizedIndicator } from "./contracts.js";
import type { normalizePineContext } from "./normalize.js";

/** Serialized, trusted VM entry. The parameters are supplied only by the trusted runtime. */
export async function runHyperVibesIndicator(args: IndicatorVmRequest, lib: typeof PineTS, normalize: typeof normalizePineContext): Promise<NormalizedIndicator> {
  const program = new lib.Indicator(args.source);
  if (program.getDeclarationType() !== "indicator") throw new Error("Only indicator() programs are supported.");
  const metadata = program.getInputsMeta();
  if (metadata.length > args.limits.inputs) throw new Error("Input declaration limit exceeded");
  const safeKey = (key: string) => !["__proto__", "constructor", "prototype"].includes(key);
  const scalar = (value: unknown): value is IndicatorInputValue => typeof value === "string" || typeof value === "boolean" || (typeof value === "number" && Number.isFinite(value));
  const counts = new Map<string, number>();
  for (const input of metadata) if (input.varId) counts.set(input.varId, (counts.get(input.varId) ?? 0) + 1);
  const inputs = metadata.map((input): IndicatorInput => {
    if (!["int", "float", "bool", "string", "source", "color"].includes(input.type)) throw new Error(`Unsupported input widget: ${input.type}`);
    const key = input.varId && counts.get(input.varId) === 1 ? input.varId : input.id;
    if (!safeKey(key) || !safeKey(input.id) || !scalar(input.defval) || input.options?.some((value) => !scalar(value))) throw new Error("Invalid indicator input metadata");
    return { key, runtimeId: input.id, variable: input.varId, title: input.title ?? input.name,
      type: input.type as IndicatorInput["type"], default: input.defval, min: input.minval, max: input.maxval, step: input.step,
      options: input.options as IndicatorInputValue[] | undefined, group: input.group, tooltip: input.tooltip };
  });
  if (new Set(inputs.map((input) => input.key)).size !== inputs.length) throw new Error("Ambiguous indicator input keys");
  for (const [key, value] of Object.entries(args.inputs)) {
    const binding = inputs.find((input) => input.key === key);
    if (!safeKey(key) || !binding || !scalar(value)) throw new Error(`Unknown or invalid input: ${key}`);
    program.input[binding.runtimeId] = value;
  }
  const resolvedInputs = Object.fromEntries(inputs.map((input) => [input.key, Object.hasOwn(args.inputs, input.key) ? args.inputs[input.key]! : input.default]));
  const pine = new lib.PineTS(args.candles, args.coin, args.pineTimeframe);
  pine.setMaxLoops(args.limits.maxLoops);
  const context = await pine.run(program);
  const result = { ...normalize(context, args), inputs, resolvedInputs };
  // QuickJS may not expose TextEncoder. Count UTF-8 bytes without host APIs or silent JSON coercion.
  const json = JSON.stringify(result);
  let bytes = 0;
  for (const char of json) { const code = char.codePointAt(0)!; bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4; }
  if (bytes > args.limits.resultBytes) throw new Error("Normalized indicator result exceeds byte limit");
  return result;
}
