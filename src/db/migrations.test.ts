import { drizzle } from "drizzle-orm/node-sqlite";
import { migrate } from "drizzle-orm/node-sqlite/migrator";
import { cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { openDatabase } from "./database.js";

const source = fileURLToPath(new URL("./migrations/", import.meta.url));

describe("generated SQLite auth migrations", () => {
  let directory: string;
  let connection: DatabaseSync;
  beforeEach(() => {
    directory = mkdtempSync(join(tmpdir(), "hypervibes-migrations-"));
    connection = openDatabase(join(directory, "test.sqlite"));
  });
  afterEach(() => {
    connection.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("applies generated SQL once, preserves records and leaves unrelated tables alone", () => {
    const db = drizzle({ client: connection });
    connection.exec("CREATE TABLE pi_test (value TEXT); INSERT INTO pi_test VALUES ('untouched')");
    const config = { migrationsFolder: source, migrationsTable: "hv_schema_migrations" };
    migrate(db, config);
    connection.exec("INSERT INTO hv_users (id, name, email) VALUES ('fixture', 'Wallet', 'fixture@example.test')");
    migrate(db, config);
    expect(connection.prepare("SELECT count(*) AS count FROM hv_schema_migrations").get()?.count).toBe(3);
    expect(connection.prepare("SELECT name FROM hv_users").get()?.name).toBe("Wallet");
    expect(connection.prepare("SELECT value FROM pi_test").get()?.value).toBe("untouched");
    expect(() => connection.exec("INSERT INTO hv_wallet_addresses (id, user_id, address, chain_id, created_at) VALUES ('wallet', 'missing', '0x0', 1, 0)")).toThrow();
  });

  it("upgrades the prior application schema without resetting existing agents", () => {
    const previous = join(directory, "previous-migrations");
    cpSync(source, previous, { recursive: true });
    rmSync(join(previous, "20261016040641_fair_paibok"), { recursive: true });
    const db = drizzle({ client: connection });
    migrate(db, { migrationsFolder: previous, migrationsTable: "hv_schema_migrations" });
    connection.exec("INSERT INTO hv_users (id, name, email) VALUES ('owner', 'Wallet', 'owner@example.test')");
    connection.exec("INSERT INTO hv_agents (id, user_id, name, creation_request_id, created_at, updated_at) VALUES ('legacy', 'owner', 'Existing agent', 'request', 1, 1)");
    migrate(db, { migrationsFolder: source, migrationsTable: "hv_schema_migrations" });
    expect(connection.prepare("SELECT name, initial_provider, initial_model_id FROM hv_agents WHERE id = 'legacy'").get()).toMatchObject({
      name: "Existing agent", initial_provider: null, initial_model_id: null,
    });
  });

  it("rolls back a failed upgrade without marking it applied or damaging existing data", () => {
    const folder = join(directory, "migrations");
    cpSync(source, folder, { recursive: true });
    const db = drizzle({ client: connection });
    const config = { migrationsFolder: folder, migrationsTable: "hv_schema_migrations" };
    migrate(db, config);
    const failing = join(folder, "20990101000000_test_failure");
    mkdirSync(failing);
    writeFileSync(join(failing, "migration.sql"), "CREATE TABLE temporary_upgrade (id TEXT);\n--> statement-breakpoint\nINSERT INTO nonexistent_table VALUES (1)");
    expect(() => migrate(db, config)).toThrow();
    expect(connection.prepare("SELECT count(*) AS count FROM hv_schema_migrations").get()?.count).toBe(3);
    expect(connection.prepare("SELECT name FROM sqlite_schema WHERE name = 'temporary_upgrade'").get()).toBeUndefined();
    expect(connection.prepare("SELECT name FROM sqlite_schema WHERE name = 'hv_users'").get()?.name).toBe("hv_users");
  });
});
