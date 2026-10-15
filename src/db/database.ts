import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { DatabaseSync } from "node:sqlite";

/** Owns a fresh-app SQLite connection; does not create domain or Durable tables. */
export function openDatabase(path: string): DatabaseSync {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const database = new DatabaseSync(path);
  try {
    // Connection-local settings must also be applied to any future connections.
    database.exec("PRAGMA busy_timeout = 5000");
    const mode = database.prepare("PRAGMA journal_mode = WAL").get();
    if (mode?.journal_mode !== "wal") {
      throw new Error("The application database requires file-backed SQLite WAL mode");
    }
    database.exec(`
      PRAGMA synchronous = FULL;
      PRAGMA foreign_keys = ON;
      PRAGMA wal_autocheckpoint = 1000;
    `);
    return database;
  } catch (error) {
    database.close();
    throw error;
  }
}
