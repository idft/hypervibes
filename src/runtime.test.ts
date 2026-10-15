import { sql } from "drizzle-orm";
import { sqliteTable, text } from "drizzle-orm/sqlite-core";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { testConfig } from "./test-config.js";
import { createRuntime } from "./runtime.js";
import type { AppRuntime } from "./runtime.js";

describe("embedded application runtime", () => {
  let directory: string;
  let runtime: AppRuntime;
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hypervibes-runtime-"));
    runtime = await createRuntime(testConfig({ APP_SQLITE_PATH: join(directory, "test.sqlite") }));
  });
  afterEach(() => {
    runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("initializes only Better Auth tables and the app migration ledger", () => {
    expect(runtime.status()).toEqual({ database: "connected", agentExecution: "not-configured" });
    const tables = runtime.db.all<{ name: string }>(sql`SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name`);
    expect(tables.map((table) => table.name)).toEqual([
      "hv_auth_accounts", "hv_auth_verifications", "hv_schema_migrations", "hv_sessions", "hv_users", "hv_wallet_addresses",
    ]);
    expect(runtime.db.get<{ synchronous: number }>(sql`PRAGMA synchronous`)?.synchronous).toBe(2);
  });

  it("supports typed Drizzle queries and synchronous transaction rollback", () => {
    // Test-only table; no Agents domain schema is created.
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
