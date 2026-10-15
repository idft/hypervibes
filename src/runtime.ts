import { sql } from "drizzle-orm";
import { drizzle } from "drizzle-orm/node-sqlite";
import { migrate } from "drizzle-orm/node-sqlite/migrator";
import { fileURLToPath } from "node:url";
import { createAuth } from "./auth.js";
import type { AppConfig } from "./config.js";
import { openDatabase } from "./db/database.js";

export interface RuntimeStatus {
  database: "connected";
  agentExecution: "not-configured";
}

/** Application-owned resources, constructed once by the host, never by a request. */
export async function createRuntime(config: AppConfig) {
  const connection = openDatabase(config.databasePath);
  try {
    const db = drizzle({ client: connection });
    migrate(db, {
      migrationsFolder: fileURLToPath(new URL("./db/migrations/", import.meta.url)),
      migrationsTable: "hv_schema_migrations",
    });
    const auth = createAuth(db, config);
    await auth.$context;
    return {
      db,
      auth,
      status(): RuntimeStatus {
        db.get(sql`SELECT 1`);
        return { database: "connected", agentExecution: "not-configured" };
      },
      close() {
        if (connection.isOpen) connection.close();
      },
    };
  } catch (error) {
    connection.close();
    throw error;
  }
}

export type AppRuntime = Awaited<ReturnType<typeof createRuntime>>;
