import type { EntryRecord, LiveState } from "@earendil-works/pi-durable";
export interface ToolActivity { id: string; name: string; status: string; arguments?: string; error?: string; durationMs?: number; queuedMs?: number; output?: string }
/** Bounded UI projection; never stream raw candle datasets or artifact bodies. */
export function toolActivity(entries: readonly EntryRecord[], live?: Readonly<LiveState>): ToolActivity[] {
  const items: ToolActivity[] = [];
  const argumentsById = new Map<string, string>();
  for (const entry of entries) for (const message of entry.model ?? []) {
    if (message.role !== "assistant") continue;
    for (const part of message.content) if (part.type === "toolCall") argumentsById.set(part.id, JSON.stringify(part.arguments).slice(0, 500));
  }
  for (const entry of entries.slice(-100)) for (const message of entry.model ?? []) {
    if (message.role !== "toolResult") continue;
    items.push({ id: message.toolCallId, name: message.toolName, status: message.isError ? "error" : "done", durationMs: message.durationMs, arguments: argumentsById.get(message.toolCallId),
      output: message.content.filter((part) => part.type === "text").map((part) => part.text).join("\n").slice(0, 1000) });
    const details = message.details;
    if (!details || typeof details !== "object" || !("calls" in details) || !Array.isArray(details.calls)) continue;
    for (const call of details.calls.slice(0, 100)) {
      if (!call || typeof call !== "object" || !("name" in call) || typeof call.name !== "string") continue;
      items.push({ id: `${message.toolCallId}${message.toolName === "codemode" ? "/c" : ":"}${"id" in call ? String(call.id) : items.length}`, name: `↳ ${call.name}`, status: "status" in call ? String(call.status) : "done",
        arguments: "args" in call ? String(call.args).slice(0, 500) : undefined, error: "error" in call ? String(call.error).slice(0, 500) : undefined,
        durationMs: "durationMs" in call && typeof call.durationMs === "number" ? call.durationMs : undefined,
        queuedMs: "queuedMs" in call && typeof call.queuedMs === "number" ? call.queuedMs : undefined });
    }
  }
  for (const slot of live?.tools ?? []) if (slot.status !== "done") {
    items.push({ id: slot.callId, name: slot.name, status: slot.status, arguments: argumentsById.get(slot.callId) });
    const details = slot.details;
    if (!details || typeof details !== "object" || !("calls" in details) || !Array.isArray(details.calls)) continue;
    // Waiting calls have no Durable nested task yet. Running calls are already
    // represented by nestedTools, so project only queued parent progress here.
    for (const call of details.calls.slice(0, 100)) {
      if (!call || typeof call !== "object" || !("status" in call) || call.status !== "queued" || !("name" in call) || typeof call.name !== "string") continue;
      items.push({ id: `${slot.callId}${slot.name === "codemode" ? "/c" : ":"}${"id" in call ? String(call.id) : items.length}`, name: `↳ ${call.name}`, status: "queued", arguments: "args" in call ? String(call.args).slice(0, 500) : undefined });
    }
  }
  for (const slot of live?.nestedTools ?? []) items.push({ id: slot.callId, name: `↳ ${slot.name}`, status: slot.summary?.isError ? "error" : slot.status,
    arguments: JSON.stringify(slot.arguments).slice(0, 500), error: slot.summary?.error?.slice(0, 500), durationMs: slot.summary?.durationMs });
  return items.slice(-125);
}
