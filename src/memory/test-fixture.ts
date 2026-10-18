import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { drizzle } from "drizzle-orm/node-sqlite";
import { migrate } from "drizzle-orm/node-sqlite/migrator";
import { openDatabase } from "../db/database.js";
import { SharedDatabase } from "../db/shared-database.js";
import { AgentError } from "../agents/agent-input.js";
import { MemoryService } from "./memory-service.js";
import { LearningService } from "./learning-service.js";
import type { MemoryActor, MemoryAuthorization } from "./operations.js";

export async function memoryFixture() {
  const directory = mkdtempSync(join(tmpdir(), "hv-memory-")), path = join(directory, "test.sqlite");
  const connection = openDatabase(path);
  migrate(drizzle({ client: connection }), { migrationsFolder: fileURLToPath(new URL("../db/migrations/", import.meta.url)), migrationsTable: "hv_schema_migrations" });
  let db = new SharedDatabase(connection);
  let now = 3600_000 * 100;
  await db.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
  for (const id of ["agent", "other"]) await db.run("INSERT INTO hv_agents(id,user_id,name,creation_request_id,created_at,updated_at) VALUES (?,'owner',?,?,0,0)", id, id, id);
  const conversations = new Map([["analyst", 1], ["reviewer", 3], ["foreign", 4]]);
  for (const [id, agent] of [["analyst", "agent"], ["reviewer", "agent"], ["foreign", "other"]]) {
    await db.run(`INSERT INTO hv_subagents(id,agent_id,kind,name,instructions,provider,model_id,tools,created_at,updated_at) VALUES (?,?,'worker',?,'instructions','faux','faux','[]',0,0)`, id!, agent!, id!);
    const config = { id: id!, agentId: agent!, kind: "worker" as const, name: id!, description: "", instructions: "Observe", provider: "faux", modelId: "faux-1", thinkingLevel: "", tools: [] as string[], templateOrigin: null, enabled: true, archived: false, delegatable: true, persistent: false, conversationId: null, schedule: "", delaySeconds: 30, nextRunAt: null, completion: "save" as const, timeoutSeconds: 900, createdAt: 0, updatedAt: 0 };
    await db.run("INSERT INTO hv_subagent_runs(id,agent_id,subagent_id,request_id,source,task,config,status,conversation_id,created_at) VALUES (?,?,?,?,'schedule','Evaluate',?,'running',?,0)", `run-${id}`, agent!, id!, `request-${id}`, JSON.stringify(config), conversations.get(id!)!);
  }
  const operator: MemoryActor = { agentId: "agent", userId: "owner", subagentId: null, runId: null, conversationId: null, toolTaskId: null };
  const actor = (id: string): MemoryActor => ({ ...operator, userId: null, subagentId: id, runId: `run-${id}`, conversationId: conversations.get(id)!, toolTaskId: 2 });
  // Explicit test-only authority. Production must supply the persisted execution
  // resolver; these services are intentionally not registered as tools yet.
  const authorize: MemoryAuthorization = async (_tx, who, capability, target) => {
    if (who.userId === "owner") return;
    if (capability === "publish_research" && who.subagentId === "analyst") return;
    if (["publish_review", "append_learning", "replace_learnings"].includes(capability) && who.subagentId === "reviewer" && (target === undefined || target === null || target === "analyst")) return;
    if (capability === "record_trade_decision" && who.subagentId === "analyst") return;
    throw new AgentError("Capability or target not permitted", 403);
  };
  const services = () => ({ memory: new MemoryService(db, authorize, () => now), learning: new LearningService(db, authorize, () => now) });
  return {
    get db() { return db; }, get now() { return now; }, set now(value: number) { now = value; }, operator, analyst: actor("analyst"), reviewer: actor("reviewer"), services,
    async reopen() { await db.close(); db = new SharedDatabase(openDatabase(path)); return services(); },
    async close() { await db.close(); rmSync(directory, { recursive: true, force: true }); },
  };
}
export function researchInput(boundary: number, patch: Record<string, unknown> = {}) {
  return { topic: "trend_setup", instrument: "BTC", interval: "1h", summary: "Ready bullish hypothesis", content: "Facts: close > EMA20 > EMA50. Interpretation: trend hypothesis.", evidenceAsOf: boundary, payload: { version: 1, bias: "long", setup: "ready", observations: ["Fixture facts"], interpretation: "Hypothesis", invalidation: ["Closed-candle alignment lost"], coverage: "complete" }, ...patch };
}
export const researchRequirement = [{ subagentId: "analyst", topic: "trend_setup", interval: "1h", maxAgeSeconds: 3600 }];
