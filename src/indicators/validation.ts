import { builtinIndicatorNames, indicatorLimits, type BuiltinIndicatorName, type IndicatorInput, type IndicatorInputs, type IndicatorInputValue } from "./contracts.js";

export class IndicatorError extends Error {
  constructor(message: string, readonly status = 400, readonly code = "indicator_invalid") { super(message); this.name = "IndicatorError"; }
}
export function parseIndicatorId(value: unknown): { kind: "builtin"; name: BuiltinIndicatorName } | { kind: "custom"; uuid: string } {
  if (typeof value !== "string") throw new IndicatorError("Invalid indicator ID");
  if (value.startsWith("builtin:")) {
    const name = builtinIndicatorNames.find((name) => value === `builtin:${name}`);
    if (name) return { kind: "builtin", name };
  }
  if (/^custom:[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)) return { kind: "custom", uuid: value.slice(7).toLowerCase() };
  throw new IndicatorError("Invalid indicator ID");
}
export function safeInputKey(key: string) { return !["__proto__", "constructor", "prototype"].includes(key); }
export function scalarInput(value: unknown): value is IndicatorInputValue {
  return typeof value === "boolean" || typeof value === "string" || (typeof value === "number" && Number.isFinite(value));
}
export function validateInputs(value: unknown): IndicatorInputs {
  if (!value || typeof value !== "object" || Array.isArray(value) || ![Object.prototype, null].includes(Object.getPrototypeOf(value))) throw new IndicatorError("Inputs must be a scalar-valued object");
  const entries = Object.entries(value);
  if (entries.length > indicatorLimits.inputs) throw new IndicatorError("Too many indicator inputs");
  for (const [key, item] of entries) {
    if (!safeInputKey(key) || !scalarInput(item)) throw new IndicatorError(`Invalid input: ${key}`);
  }
  return Object.fromEntries(entries) as IndicatorInputs;
}
/** Self-contained because the same formatter is serialized into the isolated VM. */
export function unknownIndicatorInputsMessage(unknownKeys: string[], validKeys: string[]): string {
  const quoted = (keys: string[]) => keys.map((key) => JSON.stringify(key)).join(", ");
  return `Unknown indicator input${unknownKeys.length === 1 ? "" : "s"}: ${quoted(unknownKeys)}. Valid input keys: ${validKeys.length ? quoted(validKeys) : "(none)"}. For saved/built-in indicators, use indicator_get for exact keys, types, defaults and constraints. Omit inputs to use defaults.`;
}
export function resolveInputs(schema: IndicatorInput[], overrides: IndicatorInputs): IndicatorInputs {
  validateInputs(overrides);
  const validKeys = schema.map((input) => input.key);
  const unknownKeys = Object.keys(overrides).filter((key) => !validKeys.includes(key));
  if (unknownKeys.length) throw new IndicatorError(unknownIndicatorInputsMessage(unknownKeys, validKeys));
  return Object.fromEntries(schema.map((input) => {
    const value = Object.hasOwn(overrides, input.key) ? overrides[input.key]! : input.default;
    const numeric = input.type === "int" || input.type === "float";
    if (!safeInputKey(input.key) || !scalarInput(value)
      || (numeric ? typeof value !== "number" : input.type === "bool" ? typeof value !== "boolean" : typeof value !== "string")
      || (input.type === "int" && !Number.isInteger(value))
      || (typeof value === "number" && ((input.min !== undefined && value < input.min) || (input.max !== undefined && value > input.max)))
      || (input.options && !input.options.includes(value))) throw new IndicatorError(`Invalid value for input ${input.key}`);
    return [input.key, value];
  }));
}
/** Remove strings and comments while retaining newlines and token separation. Not a Pine parser. */
export function pineCodeTokens(source: string): string {
  let result = "", quote = "", comment = false;
  for (let i = 0; i < source.length; i++) {
    const char = source[i]!;
    if (comment) { if (char === "\n") { comment = false; result += "\n"; } else result += " "; }
    else if (quote) {
      if (char === "\\") { result += "  "; i++; }
      else { if (char === quote) quote = ""; result += char === "\n" ? "\n" : " "; }
    } else if (char === '"' || char === "'") { quote = char; result += " "; }
    else if (char === "/" && source[i + 1] === "/") { comment = true; result += "  "; i++; }
    else result += char;
  }
  return result;
}
export function validateSource(source: string, maxBytes: number = indicatorLimits.sourceBytes) {
  if (typeof source !== "string" || new TextEncoder().encode(source).length > maxBytes) throw new IndicatorError(`Pine source exceeds ${maxBytes} bytes`);
  // A directive inside a string or trailing comment is not a native-source marker.
  const directive = source.split(/\r?\n/).some((line, index, lines) => /^\s*\/\/@version=[56]\s*$/.test(line) && pineCodeTokens(lines.slice(0, index).join("\n")).trim() === "");
  if (!directive) throw new IndicatorError("Native PineScript //@version=5 or //@version=6 is required");
  const tokens = pineCodeTokens(source);
  if (/\brequest\s*\./.test(tokens)) throw new IndicatorError("request.* data requests are not supported");
  if (/\b(strategy|library)\s*\(/.test(tokens) || /^\s*import\b/m.test(tokens)) throw new IndicatorError("Strategies and imported Pine libraries are not supported");
  if (!/\bindicator\s*\(/.test(tokens)) throw new IndicatorError("An indicator() declaration is required");
}
/** Anonymous IDs cannot safely migrate across a source version. Keep overrides and ask to reconfigure. */
export function validateInputMigration(previous: IndicatorInput[], next: IndicatorInput[], overrides: IndicatorInputs, sourceChanged: boolean) {
  for (const key of Object.keys(overrides)) {
    const before = previous.find((input) => input.key === key), after = next.find((input) => input.key === key);
    if (!before || !after || before.type !== after.type || (sourceChanged && (!before.variable || before.variable !== after.variable))) throw new IndicatorError("Inputs changed; reconfigure this indicator.", 409, "indicator_inputs_changed");
  }
  return resolveInputs(next, overrides);
}
