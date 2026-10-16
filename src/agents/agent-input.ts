const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export class AgentError extends Error {
  constructor(message: string, readonly status = 400) { super(message); }
}
export function agentId(value: string) {
  if (!uuid.test(value)) throw new AgentError("Agent not found", 404);
  return value.toLowerCase();
}
export function requestId(value: string) {
  if (!uuid.test(value)) throw new AgentError("A valid request ID is required");
  return value.toLowerCase();
}
export function agentName(value: string) {
  const name = value.trim();
  if (!name || name.length > 80 || [...name].some((character) => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127)) throw new AgentError("Name must contain 1–80 printable characters");
  return name;
}
export function chatMessage(value: string) {
  const message = value.trim();
  if (!message || message.length > 16_000) throw new AgentError("Message must contain 1–16,000 characters");
  return message;
}
/** Persistent storage identity: changing this format requires data migration. */
export function harnessPrefix(id: string) { return `pi_${agentId(id).replaceAll("-", "")}_`; }
