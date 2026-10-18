import { sql } from "drizzle-orm";
import { sqliteTable, text } from "drizzle-orm/sqlite-core";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { testConfig } from "./test-config.js";
import { createRuntime, type AppRuntime } from "./runtime.js";

describe("embedded application runtime", () => {
  let directory: string;
  let runtime: AppRuntime;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "test.sqlite") });
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hypervibes-runtime-"));
    runtime = await createRuntime(config());
  });
  afterEach(async () => {
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("initializes app tables, not unused Agent harnesses, with FULL durability", async () => {
    expect(runtime.status()).toEqual({ database: "connected", agentExecution: "available" });
    const tables = await runtime.database.all<{ name: string }>("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name");
    expect(tables.map((table) => table.name)).toEqual([
      "hv_agents", "hv_artifact_links", "hv_artifact_receipts", "hv_artifacts", "hv_auth_accounts", "hv_auth_verifications", "hv_exchange_nonces", "hv_hyperliquid_settings", "hv_installation", "hv_journal_accounts", "hv_journal_entries", "hv_journal_notes", "hv_journal_sync", "hv_order_actions", "hv_order_events", "hv_order_items", "hv_order_requests", "hv_provider_connections", "hv_schema_migrations", "hv_sessions", "hv_subagent_runs", "hv_subagents", "hv_users", "hv_wallet_addresses", "sqlite_sequence",
    ]);
    expect((await runtime.database.get<{ synchronous: number }>("PRAGMA synchronous"))?.synchronous).toBe(2);
  });

  it("supports typed async Drizzle queries and reserved transaction rollback", async () => {
    const records = sqliteTable("test_records", { id: text("id").primaryKey(), name: text("name").notNull() });
    await runtime.db.run(sql`CREATE TABLE test_records (id TEXT PRIMARY KEY, name TEXT NOT NULL) STRICT`);
    await runtime.db.insert(records).values({ id: "one", name: "Agent" }).run();
    expect(await runtime.db.select().from(records).all()).toEqual([{ id: "one", name: "Agent" }]);
    await expect(runtime.database.appTransaction(async (transaction) => {
      await transaction.insert(records).values({ id: "two", name: "Rolled back" }).run();
      throw new Error("rollback fixture");
    })).rejects.toThrow("rollback fixture");
    expect(await runtime.db.select().from(records).all()).toHaveLength(1);
  });

  it("closes idempotently and refuses queries after close", async () => {
    await Promise.all([runtime.close(), runtime.close()]);
    await expect(runtime.database.get("SELECT 1")).rejects.toThrow("closed");
    expect(() => runtime.status()).toThrow();
  });

  it("rejects ownership from a separate Node process", () => {
    const child = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", `
      import { createRuntime } from './src/runtime.ts';
      try {
        const runtime = await createRuntime(${JSON.stringify(config())});
        await runtime.close();
        process.exitCode = 1;
      } catch (error) {
        if (error.code !== 'ELOCKED') throw error;
        console.log('locked');
      }
    `], { cwd: process.cwd(), encoding: "utf8", timeout: 15_000 });
    expect(child.error).toBeUndefined();
    expect(child.status, child.stderr).toBe(0);
    expect(child.stdout.trim()).toBe("locked");
  });

  it("rejects another owner and releases ownership on shutdown", async () => {
    await expect(createRuntime(config())).rejects.toThrow("already being held");
    expect(runtime.status().database).toBe("connected");
    await runtime.close();
    runtime = await createRuntime(config());
    expect(runtime.status().database).toBe("connected");
  });
});
