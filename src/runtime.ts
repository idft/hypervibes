import { migrateApplication } from "./db/migrations.js";
import { AgentService, type AgentServiceOptions } from "./agents/agent-service.js";
import { createAuth } from "./auth.js";
import type { AppConfig } from "./config.js";
import { openDatabase } from "./db/database.js";
import { lockProfile } from "./db/profile-lock.js";
import { applicationDatabase, SharedDatabase } from "./db/shared-database.js";
import type { AccountFeedFactory } from "./hyperliquid/live-dashboard.js";
import { OrderService } from "./hyperliquid/orders/order-service.js";
import { HyperliquidService } from "./hyperliquid/service.js";
import { SharedHyperliquidFeeds } from "./hyperliquid/shared-feeds.js";
import { HttpTransport } from "@nktkas/hyperliquid";
import { exchangeHistory, JournalService, type HistoryFetcher, type JournalFeedFactory } from "./hyperliquid/journal-service.js";
import { ProviderService, type ProviderServiceOptions } from "./providers/provider-service.js";
import type { FastifyBaseLogger } from "fastify";
import { resolve } from "node:path";

export interface RuntimeStatus {
  database: "connected";
  agentExecution: "available";
}

/** Application-owned resources, constructed once by the host, never by a request. */
export async function createRuntime(config: AppConfig, options: AgentServiceOptions & ProviderServiceOptions & { accountFeedFactory?: AccountFeedFactory; journalFeedFactory?: JournalFeedFactory; journalHistoryFetcher?: HistoryFetcher; logger?: FastifyBaseLogger } = {}) {
  const logger = options.logger;
  const databasePath = resolve(config.databasePath);
  logger?.info({ databasePath }, "Opening SQLite database");
  const connection = openDatabase(config.databasePath);
  const database = new SharedDatabase(connection);
  let release: (() => Promise<void>) | undefined;
  let agents: AgentService | undefined;
  let hyperliquid: HyperliquidService | undefined;
  let providers: ProviderService | undefined;
  let orders: OrderService | undefined;
  let closing: Promise<void> | undefined;
  let failure: Error | undefined;
  const close = (force?: AbortSignal) => closing ??= (async () => {
    try { await agents?.close(force); }
    finally {
      try {
        try { await orders?.close(); }
        finally { await hyperliquid?.close(); }
      } finally {
        try { await providers?.close(); }
        finally {
          try { await database.close(); }
          finally { await release?.(); }
        }
      }
    }
  })();
  try {
    release = await lockProfile(config.databasePath, (error) => {
      failure = error;
      // Stop admission synchronously, then drain execution before releasing SQL.
      void close().catch((error: unknown) => {
        if (logger) logger.error(error, "Failed to close compromised application storage");
        else console.error("Failed to close compromised application storage", error);
      });
    });
    // Native migration access is allowed only before the shared queue is used.
    logger?.info({ databasePath, journalMode: "wal", synchronous: "FULL", foreignKeys: true }, "SQLite database connected and profile locked");
    const migrations = migrateApplication(connection);
    logger?.info(migrations, migrations.applied > 0 ? "Database migrations applied" : "Database schema is up to date; no migrations needed");
    const db = applicationDatabase(database);
    const auth = createAuth(db, config);
    await auth.$context;
    const providerService = new ProviderService(db, config.encryptionKey, options);
    providers = providerService;
    await providerService.initialize();
    const transport = new HttpTransport();
    const feeds = new SharedHyperliquidFeeds();
    const journal = new JournalService(database, options.journalHistoryFetcher ?? exchangeHistory(transport), logger?.child({ component: "journal" }), options.journalFeedFactory ?? feeds.journal);
    const orderService = new OrderService(database, transport, {
      account: (userId, agentId) => hyperliquid!.agentTradingAccount(userId, agentId),
      authority: (userId, agentId, signal, needsBuilder) => hyperliquid!.orderAuthority(userId, agentId, signal, needsBuilder),
    }, logger?.child({ component: "orders" }));
    orders = orderService;
    hyperliquid = new HyperliquidService(db, config.encryptionKey, transport, options.accountFeedFactory, journal, (signer) => orderService.store.nonce(signer), logger?.child({ component: "hyperliquid" }), feeds);
    await orderService.recover();
    const agentService = new AgentService(db, database, providerService, hyperliquid, orderService, { ...options, logger: logger?.child({ component: "agents" }) });
    providerService.onChanged = (userId) => agentService.providersChanged(userId);
    agents = agentService;
    await agentService.recover();
    return {
      db,
      database,
      auth,
      origin: config.authOrigin,
      agents: agentService,
      hyperliquid,
      orders: orderService,
      providers: providerService,
      start() {
        if (closing || failure) throw new Error("Runtime is unavailable");
        hyperliquid?.start();
        orderService.start();
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
