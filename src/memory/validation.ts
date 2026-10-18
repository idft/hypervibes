import { AgentError } from "../agents/agent-input.js";

export function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new AgentError("Expected an object");
  return value as Record<string, unknown>;
}
export function fields(value: unknown, keys: readonly string[]) {
  const row = object(value);
  if (Object.keys(row).some((key) => !keys.includes(key))) throw new AgentError("Unknown field; provenance and authority are host-owned");
  return row;
}
export function text(value: unknown, max: number, label: string, empty = false): string {
  if (typeof value !== "string" || value.length > max || (!empty && !value.trim())) throw new AgentError(`Invalid ${label} (maximum ${max} characters)`);
  return value.trim();
}
export function integer(value: unknown, min: number, max: number, label: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < min || value > max) throw new AgentError(`Invalid ${label}`);
  return value;
}
export function choice<const T extends readonly string[]>(value: unknown, options: T): T[number] {
  if (typeof value !== "string" || !options.includes(value)) throw new AgentError(`Expected ${options.join(" / ")}`);
  return value as T[number];
}
export function array<T>(value: unknown, max: number, parse: (item: unknown) => T): T[] {
  if (!Array.isArray(value) || value.length > max) throw new AgentError(`Expected at most ${max} items`);
  return value.map(parse);
}
export function unique<T>(values: T[]): T[] {
  if (new Set(values).size !== values.length) throw new AgentError("Duplicate values are not allowed");
  return values;
}
export function positiveDecimal(value: unknown): string {
  if (typeof value !== "string" || value.length > 80 || !/^(0|[1-9][0-9]*)(\.[0-9]+)?$/.test(value) || value.trim() !== value || !/[1-9]/.test(value)) throw new AgentError("Expected an exact positive decimal string");
  return value;
}
