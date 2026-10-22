import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { openDatabase } from "../db/database.js";
import { migrateApplication } from "../db/migrations.js";
import { SharedDatabase } from "../db/shared-database.js";
import { IndicatorStore } from "./indicator-store.js";
import type { IndicatorSaveRequest } from "./contracts.js";

const request: IndicatorSaveRequest = { name: "Test", description: "description", interpretation: "Not an order", source: '//@version=6\nindicator("Test")\nplot(close)', validation: { coin: "BTC", interval: "1h", start: 0, end: 3600_000 } };
const actor = { kind: "user" as const, userId: "owner" };
const metadata = { inputs: [], overlay: false };
let directory: string, database: SharedDatabase, store: IndicatorStore;
beforeEach(async () => {
  directory = mkdtempSync(join(tmpdir(), "hv-indicators-"));
  const connection = openDatabase(join(directory, "test.sqlite")); migrateApplication(connection);
  database = new SharedDatabase(connection); store = new IndicatorStore(database);
  await database.run("INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test')");
  for (const id of ["a", "b"]) await database.run("INSERT INTO hv_agents(id,user_id,name,creation_request_id,created_at,updated_at) VALUES (?,'owner',?,?,0,0)", id, id, id);
});
afterEach(async () => { await database.close(); rmSync(directory, { recursive: true, force: true }); });
const save = (key: string, input = request, validate = async () => metadata) => store.save("a", input, key, actor, validate);

describe("Agent-owned immutable indicator source versions", () => {
  it("stores defaults metadata, scopes reads/history, and permits duplicate names", async () => {
    const a = await save("create"), b = await save("another");
    expect(a.indicatorId).not.toBe(b.indicatorId);
    expect(await store.get("a", a.indicatorId)).toMatchObject({ source: request.source, descriptor: { version: 1, name: "Test", builtIn: false, inputs: [] } });
    expect(await store.list("a")).toHaveLength(2);
    expect(await store.list("b")).toEqual([]);
    await expect(store.get("b", a.indicatorId)).rejects.toMatchObject({ status: 404 });
    await expect(store.versions("b", a.indicatorId)).rejects.toMatchObject({ status: 404 });
    await expect(store.get("a", "builtin:ema")).rejects.toThrow("custom");
  });
  it("replays create/update receipts without validating again or returning later versions", async () => {
    const validate = vi.fn(async () => metadata);
    const first = await save("create", request, validate);
    expect(await save("create", request, validate)).toEqual(first); expect(validate).toHaveBeenCalledTimes(1);
    const update = { ...request, indicatorId: first.indicatorId, expectedVersion: 1, source: request.source + "\n// revised" };
    const second = await save("update", update, validate);
    expect(second.version).toBe(2);
    expect(await save("create", request, validate)).toEqual(first);
    expect(await save("update", update, validate)).toEqual(second);
    await expect(save("create", { ...request, description: "different" })).rejects.toMatchObject({ status: 409 });
    expect((await store.get("a", first.indicatorId, 1)).source).toBe(request.source);
    expect((await store.get("a", first.indicatorId)).source).toBe(update.source);
    expect(await store.versions("a", first.indicatorId, { limit: 1 })).toMatchObject({ versions: [{ version: 2 }], nextOffset: 1 });
  });
  it("updates metadata without a source version or VM call; exact bytes determine source changes", async () => {
    const first = await save("create");
    const validate = vi.fn(async () => metadata);
    const update = { ...request, indicatorId: first.indicatorId, expectedVersion: 1, description: "new description", validation: undefined };
    expect((await save("metadata", update, validate)).version).toBe(1);
    expect(validate).not.toHaveBeenCalled();
    expect((await store.get("a", first.indicatorId)).descriptor.description).toBe("new description");
    await expect(save("missing", { ...update, source: request.source + "\n" })).rejects.toThrow("validation window");
    expect((await save("whitespace", { ...update, source: request.source + "\n", validation: request.validation })).version).toBe(2);
  });
  it("keeps the prior source current after invalid validation and creates no receipt", async () => {
    const first = await save("create");
    await expect(save("invalid", { ...request, indicatorId: first.indicatorId, expectedVersion: 1, source: request.source + "\n// invalid runtime" }, async () => { throw new Error("Runtime failure"); })).rejects.toThrow("Runtime failure");
    expect((await store.get("a", first.indicatorId)).descriptor.version).toBe(1);
    expect(await database.get("SELECT * FROM hv_indicator_operations WHERE key='invalid'")).toBeUndefined();
  });
  it("does not hold SQL while validating; exactly one concurrent source update wins", async () => {
    const first = await save("create");
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const update = { ...request, indicatorId: first.indicatorId, expectedVersion: 1, source: request.source + "\n// next" };
    const validate = async () => { await database.get("SELECT 1"); await gate; return metadata; };
    const one = save("one", update, validate), two = save("two", update, validate);
    const results = Promise.allSettled([one, two]);
    release();
    const settled = await results;
    expect(settled.filter((result) => result.status === "fulfilled")).toHaveLength(1);
    expect(settled.filter((result) => result.status === "rejected")).toHaveLength(1);
    expect((await store.versions("a", first.indicatorId)).versions).toHaveLength(2);
  });
  it("rechecks Agent lifetime after validation and cascades application rows", async () => {
    const first = await save("create");
    const validate = async () => { await database.run("DELETE FROM hv_agents WHERE id='a'"); return metadata; };
    await expect(save("deleted", { ...request, indicatorId: first.indicatorId, expectedVersion: 1, source: request.source + "\n// next" }, validate)).rejects.toMatchObject({ status: 404 });
    for (const table of ["hv_indicators", "hv_indicator_versions", "hv_indicator_operations"]) expect(await database.all(`SELECT * FROM ${table}`)).toEqual([]);
  });
  it("enforces definition and source-version quotas without pruning history", async () => {
    const first = await save("first");
    for (let version = 2; version <= 200; version++) await save(`version-${version}`, { ...request, indicatorId: first.indicatorId, expectedVersion: version - 1, source: `${request.source}\n// version ${version}` });
    await expect(save("version-overflow", { ...request, indicatorId: first.indicatorId, expectedVersion: 200, source: request.source + "\n// overflow" })).rejects.toThrow("limit (200)");
    expect((await store.get("a", first.indicatorId, 1)).source).toBe(request.source);
    expect((await store.get("a", first.indicatorId)).descriptor.version).toBe(200);
    for (let index = 1; index < 100; index++) await save(`definition-${index}`);
    await expect(save("definition-overflow")).rejects.toThrow("limit (100)");
    expect(await store.list("a")).toHaveLength(100);
  });
  it("rejects builtin mutations and foreign-Agent updates", async () => {
    const first = await save("create");
    await expect(save("builtin", { ...request, indicatorId: "builtin:ema", expectedVersion: 1 })).rejects.toThrow("read-only");
    await expect(store.save("b", { ...request, indicatorId: first.indicatorId, expectedVersion: 1 }, "foreign", actor, async () => metadata)).rejects.toMatchObject({ status: 404 });
  });
});
