import { sql } from "drizzle-orm";
import { sqliteTable, text } from "drizzle-orm/sqlite-core";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { loadConfig } from "./config.js";
import { createRuntime } from "./runtime.js";
import type { AppRuntime } from "./runtime.js";

describe("embedded application runtime", () => {
  let directory: string;
  let runtime: AppRuntime;
  beforeEach(() => {
    directory = mkdtempSync(join(tmpdir(), "hypervibes-runtime-"));
    runtime = createRuntime(loadConfig({ APP_SQLITE_PATH: join(directory, "test.sqlite") }));
  });
  afterEach(() => {
    runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("uses Drizzle on the configured Node SQLite connection without creating tables", () => {
    expect(runtime.status()).toEqual({ database: "connected", agentExecution: "not-configured" });
    expect(runtime.db.all(sql`SELECT name FROM sqlite_schema WHERE type = 'table'`)).toEqual([]);
    expect(runtime.db.get<{ synchronous: number }>(sql`PRAGMA synchronous`)?.synchronous).toBe(2);
  });

  it("supports typed Drizzle queries and synchronous transaction rollback", () => {
    // Test-only table; application schemas/migrations remain intentionally absent.
    const agents = sqliteTable("test_agents", { id: text("id").primaryKey(), name: text("name").notNull() });
    runtime.db.run(sql`CREATE TABLE test_agents (id TEXT PRIMARY KEY, name TEXT NOT NULL) STRICT`);
    runtime.db.insert(agents).values({ id: "one", name: "Agent" }).run();
    expect(runtime.db.select().from(agents).all()).toEqual([{ id: "one", name: "Agent" }]);
    expect(() => runtime.db.transaction((transaction) => {
      transaction.insert(agents).values({ id: "two", name: "Rolled back" }).run();
      throw new Error("rollback fixture");
    })).toThrow("rollback fixture");
    expect(runtime.db.select().from(agents).all()).toHaveLength(1);
  });

  it("closes idempotently and refuses queries after close", () => {
    runtime.close();
    runtime.close();
    expect(runtime.db.$client.isOpen).toBe(false);
    expect(() => runtime.status()).toThrow();
  });
});
