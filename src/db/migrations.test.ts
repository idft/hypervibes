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
    expect(migrateApplication(connection)).toEqual({ applied: 12, total: 12 });
    expect(migrateApplication(connection)).toEqual({ applied: 0, total: 12 });
  });

  it("applies generated SQL once, preserves records and leaves unrelated tables alone", () => {
    const db = drizzle({ client: connection });
    connection.exec("CREATE TABLE pi_test (value TEXT); INSERT INTO pi_test VALUES ('untouched')");
    const config = { migrationsFolder: source, migrationsTable: "hv_schema_migrations" };
    migrate(db, config);
    connection.exec("INSERT INTO hv_users (id, name, email) VALUES ('fixture', 'Wallet', 'fixture@example.test')");
    migrate(db, config);
    expect(connection.prepare("SELECT count(*) AS count FROM hv_schema_migrations").get()?.count).toBe(12);
    expect(connection.prepare("SELECT name FROM hv_users").get()?.name).toBe("Wallet");
    expect(connection.prepare("SELECT value FROM pi_test").get()?.value).toBe("untouched");
    expect(() => connection.exec("INSERT INTO hv_wallet_addresses (id, user_id, address, chain_id, created_at) VALUES ('wallet', 'missing', '0x0', 1, 0)")).toThrow();
  });

  it("upgrades the prior application schema without resetting existing agents", () => {
    const previous = join(directory, "previous-migrations");
    cpSync(source, previous, { recursive: true });
    rmSync(join(previous, "20261022015813_pine_indicators"), { recursive: true });
    rmSync(join(previous, "20261021051229_chart_workspace"), { recursive: true });
    rmSync(join(previous, "20261016040641_provider_connections"), { recursive: true });
    rmSync(join(previous, "20261016061831_trade_journal"), { recursive: true });
    rmSync(join(previous, "20261016080320_order_execution"), { recursive: true });
    rmSync(join(previous, "20261016181110_subagents"), { recursive: true });
    rmSync(join(previous, "20261017222959_research_artifacts"), { recursive: true });
    rmSync(join(previous, "20261018061531_memories_and_insights"), { recursive: true });
    rmSync(join(previous, "20261020091156_worker_event_inbox"), { recursive: true });
    rmSync(join(previous, "20261021222116_gateways"), { recursive: true });
    const db = drizzle({ client: connection });
    migrate(db, { migrationsFolder: previous, migrationsTable: "hv_schema_migrations" });
    connection.exec("INSERT INTO hv_users (id, name, email) VALUES ('owner', 'Wallet', 'owner@example.test')");
    connection.exec("INSERT INTO hv_agents (id, user_id, name, creation_request_id, created_at, updated_at) VALUES ('legacy', 'owner', 'Existing agent', 'request', 1, 1)");
    expect(migrateApplication(connection)).toEqual({ applied: 10, total: 12 });
    expect(connection.prepare("SELECT name, initial_provider, initial_model_id FROM hv_agents WHERE id = 'legacy'").get()).toMatchObject({
      name: "Existing agent", initial_provider: null, initial_model_id: null,
    });
  });

  it("upgrades the seven-migration schema additively and enforces same-Agent audit references", () => {
    const previous = join(directory, "seven-migrations");
    cpSync(source, previous, { recursive: true });
    rmSync(join(previous, "20261022015813_pine_indicators"), { recursive: true });
    rmSync(join(previous, "20261021051229_chart_workspace"), { recursive: true });
    rmSync(join(previous, "20261018061531_memories_and_insights"), { recursive: true });
    rmSync(join(previous, "20261020091156_worker_event_inbox"), { recursive: true });
    rmSync(join(previous, "20261021222116_gateways"), { recursive: true });
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
    rmSync(join(previous, "20261022015813_pine_indicators"), { recursive: true });
    rmSync(join(previous, "20261021051229_chart_workspace"), { recursive: true });
    rmSync(join(previous, "20261020091156_worker_event_inbox"), { recursive: true });
    migrate(drizzle({ client: connection }), { migrationsFolder: previous, migrationsTable: "hv_schema_migrations" });
    connection.exec(`INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test');
      INSERT INTO hv_agents(id,user_id,name,creation_request_id,created_at,updated_at) VALUES ('a','owner','A','a',1,1),('b','owner','B','b',1,1);
      INSERT INTO hv_subagents(id,agent_id,kind,name,instructions,provider,model_id,tools,schedule,created_at,updated_at) VALUES ('worker-a','a','worker','Trader','saved custom instructions','faux','faux','[]','5m',1,1),('other-a','a','worker','Other','unchanged','faux','faux','[]','',1,1),('worker-b','b','worker','B','unchanged','faux','faux','[]','',1,1);
      INSERT INTO hv_subagent_runs(id,agent_id,subagent_id,request_id,source,task,config,status,created_at) VALUES ('old-run','a','worker-a','old-run','manual','saved task','{}','completed',1);`);
    expect(migrateApplication(connection)).toEqual({ applied: 3, total: 12 });
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

  it("creates same-Agent order provenance in the existing migration chain", () => {
    expect(migrateApplication(connection)).toEqual({ applied: 12, total: 12 });
    connection.exec(`INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test');
      INSERT INTO hv_agents(id,user_id,name,creation_request_id,created_at,updated_at) VALUES ('a','owner','A','a',1,1),('b','owner','B','b',1,1);
      INSERT INTO hv_subagents(id,agent_id,kind,name,instructions,provider,model_id,tools,created_at,updated_at) VALUES ('worker-a','a','worker','A','Observe','faux','faux','[]',1,1),('other-a','a','worker','Other','Observe','faux','faux','[]',1,1),('worker-b','b','worker','B','Observe','faux','faux','[]',1,1);
      INSERT INTO hv_subagent_runs(id,agent_id,subagent_id,request_id,source,task,config,created_at) VALUES ('run-a','a','worker-a','run-a','manual','Observe','{}',1),('run-b','b','worker-b','run-b','manual','Observe','{}',1);
      INSERT INTO hv_order_requests(id,agent_id,account,request_key,kind,input,created_at) VALUES ('request-a','a','account','a','submit','{}',1),('request-b','b','account','b','cancel','{}',1);`);
    const origin = connection.prepare("INSERT INTO hv_order_request_provenance(request_id,agent_id,producer_subagent_id,source_run_id,conversation_id,tool_task_id) VALUES (?,?,?,?,1,2)");
    expect(() => origin.run("request-a", "a", "worker-b", "run-b")).toThrow();
    expect(() => origin.run("request-b", "a", "worker-a", "run-a")).toThrow();
    expect(() => origin.run("request-a", "a", "other-a", "run-a")).toThrow();
    origin.run("request-a", "a", "worker-a", "run-a");
    expect(() => origin.run("request-a", "a", "worker-a", "run-a")).toThrow();
    expect(connection.prepare("PRAGMA foreign_key_check").all()).toEqual([]);
    expect(migrateApplication(connection)).toEqual({ applied: 0, total: 12 });
  });

  it("cascades a populated agent ownership graph without deleting independent account/user data", () => {
    migrateApplication(connection);
    connection.exec(`INSERT INTO hv_users(id,name,email) VALUES ('owner','Owner','owner@example.test');
      INSERT INTO hv_agents(id,user_id,name,creation_request_id,created_at,updated_at) VALUES ('a','owner','A','a',1,1),('b','owner','B','b',1,1);
      INSERT INTO hv_provider_connections(user_id,provider_id,auth_type,ciphertext,key_id,updated_at) VALUES ('owner','faux','api_key',X'41','key',1);
      INSERT INTO hv_journal_accounts(account) VALUES ('account');
      INSERT INTO hv_journal_entries(account,id,kind,time,data) VALUES ('account','fill','fill',1,'{}');
      INSERT INTO hv_journal_notes(id,account,target,author_id,body,created_at) VALUES ('note','account','fill','owner','Keep',1);
      INSERT INTO hv_exchange_nonces VALUES ('signer',123);`);
    for (const id of ["a", "b"]) connection.exec(`
      INSERT INTO hv_subagents(id,agent_id,kind,name,instructions,provider,model_id,tools,triggers,created_at,updated_at) VALUES ('worker-${id}','${id}','worker','Worker','Observe','faux','faux','[]','{"inboxEnabled":true,"fills":true}',1,1);
      INSERT INTO hv_subagent_runs(id,agent_id,subagent_id,request_id,source,task,config,created_at) VALUES ('run-${id}','${id}','worker-${id}','run','manual','Observe','{}',1);
      INSERT INTO hv_worker_requests(id,agent_id,subagent_id,event_key,payload,created_at,run_id) VALUES ('inbox-${id}','${id}','worker-${id}','event','{}',1,'run-${id}');
      INSERT INTO hv_subagent_run_contexts VALUES ('${id}','run-${id}','{}');
      INSERT INTO hv_memory_counters VALUES ('${id}',3);
      INSERT INTO hv_memories(agent_id,number,kind,topic,summary,content,payload,created_at,producer_subagent_id,source_run_id) VALUES ('${id}',1,'decision','test','Summary','Content','{}',1,'worker-${id}','run-${id}'),('${id}',2,'review','test','Summary','Content','{}',1,'worker-${id}','run-${id}');
      INSERT INTO hv_memory_links VALUES ('${id}',2,1,'reviews');
      INSERT INTO hv_memory_operations VALUES ('${id}','op','publish','hash','{}',1);
      INSERT INTO hv_artifacts(id,agent_id,title,description,media_type,payload,size,tags,instrument,interval,created_at) VALUES ('artifact-${id}','${id}','Evidence','','text/plain',X'41',1,'[]','','',1),('report-${id}','${id}','Report','','text/plain',X'41',1,'[]','','',1);
      INSERT INTO hv_artifact_links VALUES ('report-${id}','artifact-${id}');
      INSERT INTO hv_memory_artifacts VALUES ('${id}',1,'artifact-${id}');
      INSERT INTO hv_artifact_receipts(agent_id,operation_key,request_hash,artifact_id) VALUES ('${id}','op','hash','artifact-${id}');
      INSERT INTO hv_insight_sets VALUES ('set-${id}','${id}','worker','worker-${id}',1,1);
      INSERT INTO hv_insight_revisions(agent_id,set_id,version,content,operation,reason,created_at,producer_subagent_id,source_run_id) VALUES ('${id}','set-${id}',1,'Insight','append','Review',1,'worker-${id}','run-${id}');
      INSERT INTO hv_insight_revision_memories VALUES ('${id}','set-${id}',1,1);
      INSERT INTO hv_risk_settings(agent_id,updated_at) VALUES ('${id}',1);
      INSERT INTO hv_order_requests(id,agent_id,account,request_key,kind,input,created_at) VALUES ('request-${id}','${id}','account','request','submit','{}',1);
      INSERT INTO hv_order_actions(id,request_id,ordinal,payload,state,expires_at,updated_at) VALUES ('action-${id}','request-${id}',0,'{}','done',1,1);
      INSERT INTO hv_order_items(id,action_id,ordinal,symbol,active,view,updated_at) VALUES ('item-${id}','action-${id}',0,'BTC',0,'{}',1);
      INSERT INTO hv_order_events(item_id,source,time,data) VALUES ('item-${id}','test',1,'{}');
      INSERT INTO hv_order_request_provenance VALUES ('request-${id}','${id}','worker-${id}','run-${id}',1,2);
    `);
    // Evidence/provenance links still protect normal individual record deletion.
    expect(() => connection.exec("DELETE FROM hv_artifacts WHERE id='artifact-a'")).toThrow();
    expect(() => connection.exec("DELETE FROM hv_subagent_runs WHERE id='run-a'")).toThrow();
    connection.exec("UPDATE hv_agents SET state='stopping' WHERE id='a'; UPDATE hv_agents SET state='stopped' WHERE id='a'; DELETE FROM hv_agents WHERE id='a'");
    for (const table of ["hv_agents", "hv_subagents", "hv_subagent_runs", "hv_worker_requests", "hv_worker_subscriptions", "hv_subagent_run_contexts", "hv_memory_counters", "hv_memory_links", "hv_memory_operations", "hv_memory_artifacts", "hv_artifact_links", "hv_artifact_receipts", "hv_insight_sets", "hv_insight_revisions", "hv_insight_revision_memories", "hv_risk_settings", "hv_order_requests", "hv_order_actions", "hv_order_items", "hv_order_events", "hv_order_request_provenance"]) {
      expect(connection.prepare(`SELECT count(*) AS count FROM ${table}`).get()?.count, table).toBe(1);
    }
    expect(connection.prepare("SELECT count(*) AS count FROM hv_memories").get()?.count).toBe(2);
    expect(connection.prepare("SELECT count(*) AS count FROM hv_artifacts").get()?.count).toBe(2);
    for (const table of ["hv_users", "hv_provider_connections", "hv_journal_accounts", "hv_journal_entries", "hv_journal_notes", "hv_exchange_nonces"]) expect(connection.prepare(`SELECT count(*) AS count FROM ${table}`).get()?.count, table).toBe(1);
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
    expect(connection.prepare("SELECT count(*) AS count FROM hv_schema_migrations").get()?.count).toBe(12);
    expect(connection.prepare("SELECT name FROM sqlite_schema WHERE name = 'temporary_upgrade'").get()).toBeUndefined();
    expect(connection.prepare("SELECT name FROM sqlite_schema WHERE name = 'hv_users'").get()?.name).toBe("hv_users");
  });
});
