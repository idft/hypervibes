const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

const agentErrorBrand = Symbol.for("hypervibes.AgentError");
export class AgentError extends Error {
  readonly [agentErrorBrand] = true;
  constructor(message: string, readonly status = 400) { super(message); }
}
/** Fastify and Vite SSR load separate copies of this module in development. */
export function isAgentError(error: unknown): error is AgentError {
  return error instanceof Error && Reflect.get(error, agentErrorBrand) === true;
}
export function agentId(value: string) {
  if (!uuid.test(value) || value.trim() !== value) throw new AgentError("Agent not found", 404);
  return value.toLowerCase();
}
export function requestId(value: string) {
  if (!uuid.test(value) || value.trim() !== value) throw new AgentError("A valid request ID is required");
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
