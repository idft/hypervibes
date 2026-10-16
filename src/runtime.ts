import { drizzle } from "drizzle-orm/node-sqlite";
import { migrate } from "drizzle-orm/node-sqlite/migrator";
import { fileURLToPath } from "node:url";
import { AgentService, type AgentServiceOptions } from "./agents/agent-service.js";
import { createAuth } from "./auth.js";
import type { AppConfig } from "./config.js";
import { openDatabase } from "./db/database.js";
import { lockProfile } from "./db/profile-lock.js";
import { applicationDatabase, SharedDatabase } from "./db/shared-database.js";
import { HyperliquidService } from "./hyperliquid/service.js";
import { ProviderService, type ProviderServiceOptions } from "./providers/provider-service.js";

export interface RuntimeStatus {
  database: "connected";
  agentExecution: "available";
}

/** Application-owned resources, constructed once by the host, never by a request. */
export async function createRuntime(config: AppConfig, options: AgentServiceOptions & ProviderServiceOptions = {}) {
  const connection = openDatabase(config.databasePath);
  const database = new SharedDatabase(connection);
  let release: (() => Promise<void>) | undefined;
  let agents: AgentService | undefined;
  let hyperliquid: HyperliquidService | undefined;
  let providers: ProviderService | undefined;
  let closing: Promise<void> | undefined;
  let failure: Error | undefined;
  const close = () => closing ??= (async () => {
    try { await agents?.close(); }
    finally {
      try { await providers?.close(); }
      finally {
        try { await database.close(); }
        finally { await release?.(); }
      }
    }
  })();
  try {
    release = await lockProfile(config.databasePath, (error) => {
      failure = error;
      // Stop admission synchronously, then drain execution before releasing SQL.
      void close().catch(() => { console.error("Failed to close compromised application storage"); });
    });
    // Native migration access is allowed only before the shared queue is used.
    migrate(drizzle({ client: connection }), {
      migrationsFolder: fileURLToPath(new URL("./db/migrations/", import.meta.url)),
      migrationsTable: "hv_schema_migrations",
    });
    const db = applicationDatabase(database);
    const auth = createAuth(db, config);
    await auth.$context;
    const providerService = new ProviderService(db, config.encryptionKey, options);
    providers = providerService;
    await providerService.initialize();
    const agentService = new AgentService(db, database, providerService, options);
    providerService.onChanged = (userId) => agentService.providersChanged(userId);
    agents = agentService;
    hyperliquid = new HyperliquidService(db, config.encryptionKey);
    await agentService.recover();
    return {
      db,
      database,
      auth,
      origin: config.authOrigin,
      agents: agentService,
      hyperliquid,
      providers: providerService,
      start() {
        if (closing || failure) throw new Error("Runtime is unavailable");
        agentService.start();
      },
      status(): RuntimeStatus {
        if (closing || failure || !database.available || !connection.isOpen) throw new Error("Runtime is unavailable");
        return { database: "connected", agentExecution: "available" };
      },
      close,
    };
  } catch (error) {
    await close();
    throw error;
  }
}

export type AppRuntime = Awaited<ReturnType<typeof createRuntime>>;
