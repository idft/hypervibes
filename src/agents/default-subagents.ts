import { createHash } from "node:crypto";
import { inArray } from "drizzle-orm";
import { bodyHash, canonical } from "../canonical.js";
import { hvSubagents, type SubagentRow } from "../db/subagent-schema.js";
import type { ApplicationDatabase } from "../db/shared-database.js";
import { AgentError, agentId, requestId } from "./agent-input.js";
import { subagentInput, type SubagentInput } from "./subagent-input.js";
import { subagentTemplate, templateDraft } from "./subagent-templates.js";
import { workerTriggers } from "./worker-triggers.js";

/** Local member references exist only during import, not as persistent team membership. */
const members = [
  { key: "analyst", template: "basic-analysis", researchSources: [] },
  { key: "trader", template: "basic-trading", researchSources: ["analyst"] },
  { key: "reviewer", template: "basic-review", researchSources: [] },
] as const;

export function defaultSubagentDrafts(agent: string, request: string, model: Pick<SubagentInput, "provider" | "modelId" | "thinkingLevel">) {
  const identity = createHash("sha256").update(`${agentId(agent)}/${requestId(request)}/default-subagents`).digest("hex").slice(0, 32);
  const ids = new Map(members.map((member) => [member.key, `worker:${identity}-${member.key}`]));
  return members.map((member) => {
    const template = subagentTemplate(member.template);
    const draft = templateDraft(template.id, model);
    draft.triggers = { ...workerTriggers(draft.triggers), researchSources: member.researchSources.map((key) => ids.get(key)!) };
    return { ...subagentInput(draft), id: ids.get(member.key)!, templateOrigin: { id: template.id, version: template.version, hash: bodyHash(canonical(template)) } };
  });
}

/** Caller owns the application transaction; retries return edited/archived copies unchanged. */
export async function insertDefaultSubagents(db: ApplicationDatabase, agentId: string, request: string, model: Pick<SubagentInput, "provider" | "modelId" | "thinkingLevel">): Promise<SubagentRow[]> {
  const drafts = defaultSubagentDrafts(agentId, request, model);
  const existing = await db.select().from(hvSubagents).where(inArray(hvSubagents.id, drafts.map((draft) => draft.id))).all();
  if (existing.length) {
    if (existing.length !== drafts.length || existing.some((row) => row.agentId !== agentId)) throw new AgentError("Default sub-agent import is incomplete", 409);
    return drafts.map((draft) => existing.find((row) => row.id === draft.id)!);
  }
  const now = Date.now();
  return db.insert(hvSubagents).values(drafts.map((draft) => ({ ...draft, agentId, kind: "worker" as const, nextRunAt: null, createdAt: now, updatedAt: now }))).returning().all();
}
