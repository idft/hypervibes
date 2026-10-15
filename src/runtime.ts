import { sql } from "drizzle-orm";
import { drizzle } from "drizzle-orm/node-sqlite";
import type { AppConfig } from "./config.js";
import { openDatabase } from "./db/database.js";

export interface RuntimeStatus {
  database: "connected";
  agentExecution: "not-configured";
}

/** Application-owned resources, constructed once by the host, never by a request. */
export function createRuntime(config: AppConfig) {
  const connection = openDatabase(config.databasePath);
  try {
    const db = drizzle({ client: connection });
    return {
      db,
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

export type AppRuntime = ReturnType<typeof createRuntime>;
