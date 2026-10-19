import { drizzle } from "drizzle-orm/node-sqlite";
import { migrate } from "drizzle-orm/node-sqlite/migrator";
import type { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";

/** Startup-only native access, before the application's shared SQL queue is used. */
export function migrateApplication(connection: DatabaseSync, migrationsFolder = fileURLToPath(new URL("./migrations/", import.meta.url))) {
  const hasHistory = connection.prepare("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'hv_schema_migrations'").get();
  const migrationCount = () => Number(connection.prepare("SELECT count(*) AS count FROM hv_schema_migrations").get()?.count);
  const before = hasHistory ? migrationCount() : 0;
  migrate(drizzle({ client: connection }), { migrationsFolder, migrationsTable: "hv_schema_migrations" });
  const total = migrationCount();
  return { applied: total - before, total };
}
