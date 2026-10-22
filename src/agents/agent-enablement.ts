import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { AgentError } from "./agent-input.js";

/** Execution permission is independent of initialization/deletion lifecycle. */
export async function agentExecutable(db: SqliteExecutor, id: string): Promise<boolean> {
  const agent = await db.get<{ enabled: number; state: string }>("SELECT enabled,state FROM hv_agents WHERE id=?", id);
  return agent?.enabled === 1 && agent.state === "ready";
}
export async function assertAgentExecutable(db: SqliteExecutor, id: string): Promise<void> {
  if (!await agentExecutable(db, id)) throw new AgentError("Agent is disabled or unavailable. Enable it before starting work.", 409);
}
