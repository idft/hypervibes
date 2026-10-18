import type { EntryRecord } from "@earendil-works/pi-durable";
export interface ExecutionWarning { code: "entry_blocked" | "protection_unverified" | "uncertain_outcome"; message: string }
/** Bounded projection from public tool results, never model claims of success. */
export function executionWarnings(entries: readonly EntryRecord[]): ExecutionWarning[] {
  const warnings: ExecutionWarning[] = [];
  for (const entry of entries.slice(-100)) for (const message of entry.model ?? []) {
    if (message.role !== "toolResult" || !["submit_orders", "get_protection_status", "get_trading_context"].includes(message.toolName)) continue;
    const text = message.content.filter((part) => part.type === "text").map((part) => part.text).join("\n");
    if (message.isError) { warnings.push({ code: message.toolName === "submit_orders" ? "entry_blocked" : "protection_unverified", message: text.slice(0, 500) || "Tool failed; inspect the full result before further exposure." }); continue; }
    try {
      if (text.length > 64000) throw new Error("Over summary budget");
      const result = JSON.parse(text) as Record<string, unknown>;
      if (message.toolName === "get_trading_context" && result.eligibleForAutomaticEntry === false) warnings.push({ code: "entry_blocked", message: "Mandatory current research does not authorize automatic new exposure. This does not forbid a verified user-directed request or reduce-only management." });
      if (result.uncertain === true) warnings.push({ code: "uncertain_outcome", message: "An exchange action has an unknown outcome. Reconcile the original request ID; never blindly retry with a new ID." });
      if (message.toolName === "get_protection_status" && Array.isArray(result.positions) && result.positions.some((position: unknown) => position && typeof position === "object" && Reflect.get(position, "covered") !== true)) warnings.push({ code: "protection_unverified", message: "At least one actual position lacks verified full-size active TP and market SL. Inspect individual orders and repair deliberately; no blind duplicates, cancel-all or unapproved flattening." });
      if (message.toolName === "submit_orders" && Array.isArray(result.warnings)) for (const warning of result.warnings.slice(0, 5)) if (typeof warning === "string") warnings.push({ code: "protection_unverified", message: warning.slice(0, 500) });
    } catch { warnings.push({ code: "protection_unverified", message: "Order/protection result cannot be summarized reliably. Inspect full outcomes; completion/acceptance is not verified position protection." }); }
  }
  return warnings.slice(-10);
}
