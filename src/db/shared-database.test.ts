import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { createRegistry, Harness } from "@earendil-works/pi-durable";
import { SqliteStorage } from "@earendil-works/pi-durable/storage/sqlite";
import { sql } from "drizzle-orm";
import { integer, sqliteTable, text } from "drizzle-orm/sqlite-core";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { openDatabase } from "./database.js";
import { applicationDatabase, SharedDatabase } from "./shared-database.js";

const records = sqliteTable("test_records", { id: text().primaryKey(), value: integer().notNull() });

describe("shared SQLite facade", () => {
  it("fails closed when rollback cannot restore transaction ownership", async () => {
    const directory = mkdtempSync(join(tmpdir(), "hv-shared-failure-"));
    const connection = openDatabase(join(directory, "test.sqlite"));
    const database = new SharedDatabase(connection);
    try {
      const transaction = database.transaction(async (tx) => {
        // Deliberately violate the executor contract to force COMMIT and its
        // recovery ROLLBACK to fail, without depending on a disk malfunction.
        await tx.exec("ROLLBACK");
      });
      const unrelated = database.get("SELECT 1");
      await Promise.all([
        expect(transaction).rejects.toThrow("SQLite rollback failed"),
        expect(unrelated).rejects.toThrow("SQLite rollback failed"),
      ]);
      expect(database.available).toBe(false);
      await expect(database.get("SELECT 1")).rejects.toThrow("SQLite rollback failed");
    } finally {
      await database.close();
      expect(connection.isOpen).toBe(false);
      rmSync(directory, { recursive: true, force: true });
    }
  });
  it("serializes async transactions, expires handles, maps Drizzle rows and preserves other harnesses", async () => {
    const directory = mkdtempSync(join(tmpdir(), "hv-shared-"));
    const database = new SharedDatabase(openDatabase(join(directory, "test.sqlite")));
    const db = applicationDatabase(database);
    const harnesses: Harness[] = [];
    try {
      await database.exec("CREATE TABLE test_records (id TEXT PRIMARY KEY, value INTEGER NOT NULL)");
      await db.insert(records).values({ id: "first", value: 1 }).run();
      expect(await db.select().from(records).get()).toEqual({ id: "first", value: 1 });
      expect(await db.select().from(records).where(sql`id = 'missing'`).get()).toBeUndefined();
      expect(await db.select({ alias: records.value }).from(records).get()).toEqual({ alias: 1 });
      let release!: () => void;
      const gate = new Promise<void>((resolve) => { release = resolve; });
      let started!: () => void;
      const entered = new Promise<void>((resolve) => { started = resolve; });
      const rollback = database.appTransaction(async (transaction) => {
        await transaction.insert(records).values({ id: "rollback", value: 2 }).run();
        started();
        await gate;
        throw new Error("fixture rollback");
      });
      const rejected = expect(rollback).rejects.toThrow("fixture rollback");
      await entered;
      let outsideFinished = false;
      const outside = db.insert(records).values({ id: "outside", value: 3 }).run().then(() => { outsideFinished = true; });
      await Promise.resolve();
      expect(outsideFinished).toBe(false);
      release();
      await rejected;
      await outside;
      expect(await db.select().from(records).all()).toEqual([{ id: "first", value: 1 }, { id: "outside", value: 3 }]);
      await expect(db.transaction(async () => {})).rejects.toThrow("Failed query: begin");
      let expired!: Parameters<Parameters<SharedDatabase["transaction"]>[0]>[0];
      await database.transaction(async (tx) => { expired = tx; });
      await expect(expired.get("SELECT 1")).rejects.toThrow("closed");
      for (const prefix of ["pi_one_", "pi_two_"]) {
        harnesses.push(await Harness.open(await SqliteStorage.open(database.borrow(), { prefix }), {
          models: createModels(), registry: createRegistry(),
        }, context));
      }
      const [one, two] = await Promise.all(harnesses.map((harness) => harness.root(context)));
      await one!.configure({ instructions: "Only in harness one" }, context);
      expect((await two!.agent(context)).instructions).not.toBe("Only in harness one");
      await harnesses[0]!.close(context);
      expect((await harnesses[1]!.root(context)).id).toBe(two!.id);
      await db.insert(records).values({ id: "after-close", value: 4 }).run();
      expect(await db.select().from(records).all()).toHaveLength(3);
      expect((await database.get<{ synchronous: number }>("PRAGMA synchronous"))?.synchronous).toBe(2);
      await expect(database.appTransaction(async (transaction) => { await transaction.run(sql`INSERT INTO test_records VALUES ('bad', NULL)`); })).rejects.toThrow();
    } finally {
      await Promise.all(harnesses.map((harness) => harness.close(context)));
      await database.close();
      rmSync(directory, { recursive: true, force: true });
    }
  });
});
