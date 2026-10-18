import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { DatabaseSync } from "node:sqlite";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { openDatabase } from "./database.js";

describe("SQLite lifecycle", () => {
  let directory: string;
  let connections: DatabaseSync[];
  const open = () => {
    const database = openDatabase(join(directory, "profile", "test.sqlite"));
    connections.push(database);
    return database;
  };
  beforeEach(() => {
    directory = mkdtempSync(join(tmpdir(), "hypervibes-db-"));
    connections = [];
  });
  afterEach(() => {
    for (const database of connections) {
      if (database.isOpen) database.close();
    }
    rmSync(directory, { recursive: true, force: true });
  });

  it("creates a file with WAL, FULL durability and connection-local safety settings", () => {
    const database = open();
    expect(database.prepare("PRAGMA auto_vacuum").get()?.auto_vacuum).toBe(2);
    expect(database.prepare("PRAGMA journal_mode").get()?.journal_mode).toBe("wal");
    expect(database.prepare("PRAGMA synchronous").get()?.synchronous).toBe(2);
    expect(database.prepare("PRAGMA foreign_keys").get()?.foreign_keys).toBe(1);
    expect(database.prepare("PRAGMA busy_timeout").get()?.timeout).toBe(5000);
    expect(database.prepare("PRAGMA wal_autocheckpoint").get()?.wal_autocheckpoint).toBe(1000);
    expect(database.prepare("SELECT name FROM sqlite_schema WHERE type = 'table'").all()).toEqual([]);
  });

  it("persists application data across close/reopen and configures every connection", () => {
    const first = open();
    first.exec("CREATE TABLE test_records (value TEXT NOT NULL) STRICT");
    first.prepare("INSERT INTO test_records VALUES (?)").run("persisted");
    const second = open();
    expect(second.prepare("SELECT value FROM test_records").get()?.value).toBe("persisted");
    expect(second.prepare("PRAGMA foreign_keys").get()?.foreign_keys).toBe(1);
    expect(second.prepare("PRAGMA synchronous").get()?.synchronous).toBe(2);
    first.close();
    second.close();
    expect(open().prepare("SELECT value FROM test_records").get()?.value).toBe("persisted");
  });

  it("does not rebuild an existing NONE profile on startup", () => {
    const first = open();
    first.exec("PRAGMA auto_vacuum=NONE; VACUUM; CREATE TABLE existing_data (value TEXT); INSERT INTO existing_data VALUES ('keep')");
    first.close();
    const restored = open();
    expect(restored.prepare("PRAGMA auto_vacuum").get()?.auto_vacuum).toBe(0);
    expect(restored.prepare("SELECT value FROM existing_data").get()?.value).toBe("keep");
  });

  it("enforces foreign keys and supports rollback", () => {
    const database = open();
    database.exec(`
      CREATE TABLE test_parent (id TEXT PRIMARY KEY) STRICT;
      CREATE TABLE test_child (parent_id TEXT REFERENCES test_parent(id)) STRICT;
    `);
    expect(() => database.exec("INSERT INTO test_child VALUES ('missing')")).toThrow();
    database.exec("BEGIN IMMEDIATE; INSERT INTO test_parent VALUES ('temporary'); ROLLBACK");
    expect(database.prepare("SELECT * FROM test_parent").all()).toEqual([]);
  });

  it("rejects non-file-backed storage instead of silently losing WAL guarantees", () => {
    expect(() => openDatabase(":memory:")).toThrow("file-backed SQLite WAL");
  });
});
