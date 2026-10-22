import { randomUUID } from "node:crypto";
import { and, desc, eq } from "drizzle-orm";
import type { SqliteExecutor } from "@earendil-works/pi-durable/storage/sqlite";
import { bodyHash, canonical } from "../canonical.js";
import { hvIndicators, hvIndicatorOperations, hvIndicatorVersions } from "../db/indicator-schema.js";
import { applicationDatabase, type SharedDatabase } from "../db/shared-database.js";
import { indicatorEngine, indicatorLimits, indicatorWrapperVersion, type IndicatorActor, type IndicatorDescriptor, type IndicatorInput, type IndicatorSaveRequest } from "./contracts.js";import { IndicatorError, parseIndicatorId, validateSource } from "./validation.js";

interface DeclarationMetadata { inputs: IndicatorInput[]; overlay: boolean }
export interface StoredIndicator { descriptor: IndicatorDescriptor; source: string; createdAt: number }
export interface IndicatorSaveResult { indicatorId: string; version: number; sourceHash: string }
export function indicatorPage(offset = 0, limit: number = indicatorLimits.listLimit) {
  if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(limit) || limit < 1 || limit > indicatorLimits.maxListLimit) throw new IndicatorError("Invalid indicator page");
  return { offset, limit };
}
function validateSave(request: IndicatorSaveRequest) {
  for (const [value, max, field] of [[request.name, 100, "name"], [request.description, 2000, "description"], [request.interpretation, 8000, "interpretation"]] as const) {
    if (typeof value !== "string" || value.length > max || (field === "name" && !value.trim())) throw new IndicatorError(`Invalid indicator ${field}`);
  }
  validateSource(request.source);
  if (request.indicatorId) {
    if (parseIndicatorId(request.indicatorId).kind !== "custom") throw new IndicatorError("Built-in indicators are read-only");
    if (!Number.isSafeInteger(request.expectedVersion) || request.expectedVersion! < 1) throw new IndicatorError("expectedVersion is required on updates");
  } else if (request.expectedVersion !== undefined) throw new IndicatorError("New indicators cannot specify expectedVersion");
}
/** Application-owned immutable source history. Network/VM validation occurs outside the short SQL transaction. */
export class IndicatorStore {
  private readonly db;
  constructor(private readonly database: SharedDatabase) { this.db = applicationDatabase(database); }
  private async agentTx(tx: Pick<SqliteExecutor, "get">, agentId: string) {
    const agent = await tx.get<{ state: string }>("SELECT state FROM hv_agents WHERE id=?", agentId);
    if (!agent) throw new IndicatorError("Agent not found", 404);
    if (agent.state === "stopping" || agent.state === "stopped") throw new IndicatorError("Agent is being deleted", 409);
  }
  async assertAgent(agentId: string) { await this.agentTx(this.database, agentId); }
  async list(agentId: string) {
    await this.assertAgent(agentId);
    // A hard 100-definition quota bounds this metadata-only merge with the static catalog.
    const definitions = await this.db.select().from(hvIndicators).where(eq(hvIndicators.agentId, agentId)).orderBy(desc(hvIndicators.updatedAt)).all();
    return Promise.all(definitions.map((definition) => this.get(agentId, `custom:${definition.id}`).then((entry) => entry.descriptor)));
  }
  async get(agentId: string, id: string, version?: number): Promise<StoredIndicator> {
    const parsed = parseIndicatorId(id);
    if (parsed.kind !== "custom") throw new IndicatorError("Not a custom indicator");
    if (version !== undefined && (!Number.isSafeInteger(version) || version < 1)) throw new IndicatorError("Invalid indicator version");
    await this.assertAgent(agentId);
    const definition = await this.db.select().from(hvIndicators).where(and(eq(hvIndicators.id, parsed.uuid), eq(hvIndicators.agentId, agentId))).get();
    if (!definition) throw new IndicatorError("Indicator not found", 404);
    const saved = await this.db.select().from(hvIndicatorVersions).where(and(eq(hvIndicatorVersions.indicatorId, definition.id), eq(hvIndicatorVersions.version, version ?? definition.currentVersion))).get();
    if (!saved) throw new IndicatorError("Indicator version not found", 404);
    const metadata = JSON.parse(saved.metadata) as DeclarationMetadata;
    return { source: saved.source, createdAt: saved.createdAt, descriptor: { id: `custom:${definition.id}`, builtIn: false, name: definition.name, description: definition.description, interpretation: definition.interpretation,
      version: saved.version, sourceHash: saved.sourceHash, inputs: metadata.inputs, overlay: metadata.overlay } };
  }
  async versions(agentId: string, id: string, options: { offset?: number; limit?: number } = {}) {
    await this.get(agentId, id);
    const parsed = parseIndicatorId(id);
    if (parsed.kind !== "custom") throw new IndicatorError("Not a custom indicator");
    const { offset, limit } = indicatorPage(options.offset, options.limit);
    const rows = await this.db.select({ version: hvIndicatorVersions.version, sourceHash: hvIndicatorVersions.sourceHash, createdAt: hvIndicatorVersions.createdAt, actorKind: hvIndicatorVersions.actorKind }).from(hvIndicatorVersions)
      .where(eq(hvIndicatorVersions.indicatorId, parsed.uuid)).orderBy(desc(hvIndicatorVersions.version)).offset(offset).limit(limit + 1).all();
    return { versions: rows.slice(0, limit), nextOffset: rows.length > limit ? offset + limit : null };
  }
  async save(agentId: string, request: IndicatorSaveRequest, operationKey: string, actor: IndicatorActor, validate: () => Promise<DeclarationMetadata>, signal?: AbortSignal, engineVersion: string = indicatorEngine.version): Promise<IndicatorSaveResult> {
    validateSave(request);
    if (!operationKey || operationKey.length > 200) throw new IndicatorError("Invalid indicator operation key");
    await this.assertAgent(agentId);
    const hash = bodyHash(canonical(request));
    const replay = async () => {
      const receipt = await this.db.select().from(hvIndicatorOperations).where(and(eq(hvIndicatorOperations.agentId, agentId), eq(hvIndicatorOperations.key, operationKey))).get();
      if (!receipt) return undefined;
      if (receipt.hash !== hash) throw new IndicatorError("Operation key was used with different content", 409);
      return JSON.parse(receipt.result) as IndicatorSaveResult;
    };
    const previousReceipt = await replay();
    if (previousReceipt) return previousReceipt;
    const previous = request.indicatorId ? await this.get(agentId, request.indicatorId) : undefined;
    if (previous && request.expectedVersion !== previous.descriptor.version) throw new IndicatorError("Indicator changed; reread before saving", 409);
    const sourceChanged = !previous || previous.source !== request.source;
    if (sourceChanged && !request.validation) throw new IndicatorError("A real closed-candle validation window is required for source changes");
    signal?.throwIfAborted();
    const metadata = sourceChanged ? await validate() : { inputs: previous.descriptor.inputs, overlay: previous.descriptor.overlay };
    signal?.throwIfAborted();
    const uuid = previous ? parseIndicatorId(previous.descriptor.id) : undefined;
    const id = uuid?.kind === "custom" ? uuid.uuid : randomUUID();
    const sourceHash = bodyHash(request.source), now = Date.now();
    return this.database.appTransaction(async (db, tx) => {
      signal?.throwIfAborted();
      await this.agentTx(tx, agentId);
      const receipt = await db.select().from(hvIndicatorOperations).where(and(eq(hvIndicatorOperations.agentId, agentId), eq(hvIndicatorOperations.key, operationKey))).get();
      if (receipt) {
        if (receipt.hash !== hash) throw new IndicatorError("Operation key was used with different content", 409);
        return JSON.parse(receipt.result) as IndicatorSaveResult;
      }
      const current = await db.select().from(hvIndicators).where(and(eq(hvIndicators.id, id), eq(hvIndicators.agentId, agentId))).get();
      if (previous && (!current || current.currentVersion !== request.expectedVersion)) throw new IndicatorError("Indicator changed; reread before saving", 409);
      const version = current ? current.currentVersion + (sourceChanged ? 1 : 0) : 1;
      if (version > indicatorLimits.versions) throw new IndicatorError("Indicator source-version limit (200) reached", 409);
      if (!current) {
        const quota = await tx.get<{ count: number }>("SELECT count(*) AS count FROM hv_indicators WHERE agent_id=?", agentId);
        if ((quota?.count ?? 0) >= indicatorLimits.customPerAgent) throw new IndicatorError("Custom indicator limit (100) reached", 409);
        await db.insert(hvIndicators).values({ id, agentId, name: request.name, description: request.description, interpretation: request.interpretation, currentVersion: version, createdAt: now, updatedAt: now }).run();
      } else await db.update(hvIndicators).set({ name: request.name, description: request.description, interpretation: request.interpretation, currentVersion: version, updatedAt: now }).where(eq(hvIndicators.id, id)).run();
      if (sourceChanged) await db.insert(hvIndicatorVersions).values({ indicatorId: id, version, source: request.source, sourceHash, metadata: JSON.stringify(metadata), engineVersion, wrapperVersion: indicatorWrapperVersion,
        createdAt: now, actorKind: actor.kind, userId: actor.userId, subagentId: actor.subagentId, runId: actor.runId, conversationId: actor.conversationId, taskId: actor.taskId }).run();
      const result: IndicatorSaveResult = { indicatorId: `custom:${id}`, version, sourceHash };
      await db.insert(hvIndicatorOperations).values({ agentId, key: operationKey, hash, result: JSON.stringify(result) }).run();
      return result;
    });
  }
}
