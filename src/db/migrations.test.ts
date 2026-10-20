import { drizzle } from "drizzle-orm/node-sqlite";
import { migrate } from "drizzle-orm/node-sqlite/migrator";
import { cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { openDatabase } from "./database.js";
import { migrateApplication } from "./migrations.js";

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

  it("reports applied migrations for a fresh database and none on subsequent startups", () => {
    expect(migrateApplication(connection)).toEqual({ applied: 9, total: 9 });
    expect(migrateApplication(connection)).toEqual({ applied: 0, total: 9 });
  });

  it("applies generated SQL once, preserves records and leaves unrelated tables alone", () => {
    const db = drizzle({ client: connection });
    connection.exec("CREATE TABLE pi_test (value TEXT); INSERT INTO pi_test VALUES ('untouched')");
    const config = { migrationsFolder: source, migrationsTable: "hv_schema_migrations" };
    migrate(db, config);
    connection.exec("INSERT INTO hv_users (id, name, email) VALUES ('fixture', 'Wallet', 'fixture@example.test')");
    migrate(db, config);
    expect(connection.prepare("SELECT count(*) AS count FROM hv_schema_migrations").get()?.count).toBe(9);
    expect(connection.prepare("SELECT name FROM hv_users").get()?.name).toBe("Wallet");
    expect(connection.prepare("SELECT value FROM pi_test").get()?.value).toBe("untouched");
    expect(() => connection.exec("INSERT INTO hv_wallet_addresses (id, user_id, address, chain_id, created_at) VALUES ('wallet', 'missing', '0x0', 1, 0)")).toThrow();
  });

  it("upgrades the prior application schema without resetting existing agents", () => {
    const previous = join(directory, "previous-migrations");
    cpSync(source, previous, { recursive: true });
    rmSync(join(previous, "20261016040641_provider_connections"), { recursive: true });
    rmSync(join(previous, "20261016061831_trade_journal"), { recursive: true });
    rmSync(join(previous, "20261016080320_order_execution"), { recursive: true });
    rmSync(join(previous, "20261016181110_subagents"), { recursive: true });
    rmSync(join(previous, "20261017222959_research_artifacts"), { recursive: true });
    rmSync(join(previous, "20261018061531_memories_and_insights"), { recursive: true });
    rmSync(join(previous, "20261020091156_worker_event_inbox"), { recursive: true });
    const db = drizzle({ client: connection });
    migrate(db, { migrationsFolder: previous, migrationsTable: "hv_schema_migrations" });
    connection.exec("INSERT INTO hv_users (id, name, email) VALUES ('owner', 'Wallet', 'owner@example.test')");
    connection.exec("INSERT INTO hv_agents (id, user_id, name, creation_request_id, created_at, updated_at) VALUES ('legacy', 'owner', 'Existing agent', 'request', 1, 1)");
    expect(migrateApplication(connection)).toEqual({ applied: 7, total: 9 });
    expect(connection.prepare("SELECT name, initial_provider, initial_model_id FROM hv_agents WHERE id = 'legacy'").get()).toMatchObject({
      name: "Existing agent", initial_provider: null, initial_model_id: null,
    });
  });

  it("upgrades the seven-migration schema additively and enforces same-Agent audit references", () => {
    const previous = join(directory, "seven-migrations");
    cpSync(source, previous, { recursive: true });
    rmSync(join(previous, "20261018061531_memories_and_insights"), { recursive: true });
    rmSync(join(previous, "20261020091156_worker_event_inbox"), { recursive: true });
    const db = drizzle({ client: connection });
    migrate(db, { migrationsFolder: previous, migrationsTable: "hv_schema_migrations" });
    expect(connection.prepare("SELECT count(*) AS count FROM hv_schema_migrations").get()?.count).toBe(7);
    connection.exec(`INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test');
      INSERT INTO hv_agents(id,user_id,name,creation_request_id,created_at,updated_at) VALUES ('a','owner','A','a',1,1),('b','owner','B','b',1,1);
      INSERT INTO hv_subagents(id,agent_id,kind,name,instructions,provider,model_id,tools,created_at,updated_at) VALUES ('worker-b','b','worker','B','saved instructions','faux','faux','[]',1,1);
      INSERT INTO hv_subagent_runs(id,agent_id,subagent_id,request_id,source,task,config,created_at) VALUES ('run-b','b','worker-b','run-b','schedule','saved task','{}',1);
      INSERT INTO hv_artifacts(id,agent_id,title,description,media_type,payload,size,tags,instrument,interval,created_at) VALUES ('evidence-b','b','Evidence','','text/plain',X'41',1,'[]','','',1);
      CREATE TABLE pi_opaque_fixture (value TEXT); INSERT INTO pi_opaque_fixture VALUES ('unchanged');`);
    migrate(db, { migrationsFolder: source, migrationsTable: "hv_schema_migrations" });
    migrate(db, { migrationsFolder: source, migrationsTable: "hv_schema_migrations" });
    expect(connection.prepare("SELECT task,config FROM hv_subagent_runs").get()).toMatchObject({ task: "saved task", config: "{}" });
    expect(connection.prepare("SELECT instructions,enabled,schedule FROM hv_subagents").get()).toMatchObject({ instructions: "saved instructions", enabled: 1, schedule: "" });
    expect(connection.prepare("SELECT value FROM pi_opaque_fixture").get()?.value).toBe("unchanged");
    expect(connection.prepare("SELECT count(*) AS count FROM hv_insight_revisions").get()?.count).toBe(0);
    const publish = connection.prepare("INSERT INTO hv_memories(agent_id,number,kind,topic,summary,content,payload,created_at,producer_subagent_id,source_run_id) VALUES (?,?,'decision','test','Summary','Content','{}',1,?,?)");
    expect(() => publish.run("a", 1, "worker-b", "run-b")).toThrow();
    publish.run("a", 3, null, null);
    publish.run("b", 2, "worker-b", "run-b");
    expect(() => connection.exec("INSERT INTO hv_memory_links VALUES ('a',3,2,'based_on')")).toThrow();
    expect(() => connection.exec("INSERT INTO hv_memory_artifacts VALUES ('a',3,'evidence-b')")).toThrow();
    expect(() => connection.exec("INSERT INTO hv_insight_sets VALUES ('wrong','a','worker','worker-b',0,1)")).toThrow();
    connection.exec("INSERT INTO hv_insight_sets VALUES ('general','a','agent',NULL,0,1)");
    expect(() => connection.exec("INSERT INTO hv_insight_sets VALUES ('duplicate','a','agent',NULL,0,1)")).toThrow();
    expect(connection.prepare("PRAGMA foreign_key_check").all()).toEqual([]);
  });

  it("adds the inbox without changing saved schedules, instructions, or existing run configuration", () => {
    const previous = join(directory, "eight-migrations");
    cpSync(source, previous, { recursive: true });
    rmSync(join(previous, "20261020091156_worker_event_inbox"), { recursive: true });
    migrate(drizzle({ client: connection }), { migrationsFolder: previous, migrationsTable: "hv_schema_migrations" });
    connection.exec(`INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test');
      INSERT INTO hv_agents(id,user_id,name,creation_request_id,created_at,updated_at) VALUES ('a','owner','A','a',1,1),('b','owner','B','b',1,1);
      INSERT INTO hv_subagents(id,agent_id,kind,name,instructions,provider,model_id,tools,schedule,created_at,updated_at) VALUES ('worker-a','a','worker','Trader','saved custom instructions','faux','faux','[]','5m',1,1),('other-a','a','worker','Other','unchanged','faux','faux','[]','',1,1),('worker-b','b','worker','B','unchanged','faux','faux','[]','',1,1);
      INSERT INTO hv_subagent_runs(id,agent_id,subagent_id,request_id,source,task,config,status,created_at) VALUES ('old-run','a','worker-a','old-run','manual','saved task','{}','completed',1);`);
    expect(migrateApplication(connection)).toEqual({ applied: 1, total: 9 });
    expect(connection.prepare("SELECT instructions,schedule,triggers FROM hv_subagents WHERE id='worker-a'").get()).toMatchObject({ instructions: "saved custom instructions", schedule: "5m", triggers: '{"inboxEnabled":false,"workerDelegates":[],"researchSources":[],"fills":false,"orderChanges":false}' });
    expect(connection.prepare("SELECT config FROM hv_subagent_runs").get()?.config).toBe("{}");
    connection.exec(`UPDATE hv_subagents SET triggers='{"inboxEnabled":true}' WHERE id IN ('worker-a','other-a')`);
    expect(connection.prepare("SELECT count(*) AS count FROM hv_subagents WHERE agent_id='a' AND json_extract(triggers, '$.inboxEnabled')=1").get()?.count).toBe(2);
    const request = connection.prepare("INSERT INTO hv_worker_requests(id,agent_id,subagent_id,event_key,payload,created_at,run_id) VALUES (?,?,?,'event','{}',1,?)");
    expect(() => request.run("wrong-agent", "a", "worker-b", null)).toThrow();
    expect(() => request.run("wrong-worker", "a", "other-a", "old-run")).toThrow();
    request.run("valid", "a", "worker-a", "old-run");
    expect(connection.prepare("PRAGMA foreign_key_check").all()).toEqual([]);
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
    expect(connection.prepare("SELECT count(*) AS count FROM hv_schema_migrations").get()?.count).toBe(9);
    expect(connection.prepare("SELECT name FROM sqlite_schema WHERE name = 'temporary_upgrade'").get()).toBeUndefined();
    expect(connection.prepare("SELECT name FROM sqlite_schema WHERE name = 'hv_users'").get()?.name).toBe("hv_users");
  });
});
