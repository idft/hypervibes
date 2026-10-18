import { createHash } from "node:crypto";

/** Stable JSON object-key ordering for persisted request identity. */
export function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") return `{${Object.entries(value).filter(([, v]) => v !== undefined).sort(([a], [b]) => a.localeCompare(b)).map(([k, v]) => `${JSON.stringify(k)}:${canonical(v)}`).join(",")}}`;
  return JSON.stringify(value);
}

/** Stable SHA-256 digest of canonical content, for persisted hashes. */
export function bodyHash(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}