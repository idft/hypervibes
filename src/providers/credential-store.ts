import type { AuthOperationOptions, Credential, CredentialStore } from "@earendil-works/pi-ai";
import type { ModelsStore, ModelsStoreEntry } from "@earendil-works/pi-ai";
import { and, eq } from "drizzle-orm";
import type { EncryptionKeySetting } from "../config.js";
import { hvProviderConnections } from "../db/provider-schema.js";
import type { ApplicationDatabase } from "../db/shared-database.js";
import { decryptSecret, encryptSecret } from "../hyperliquid/crypto.js";
import { AgentError } from "../agents/agent-input.js";

/** One instance per user, shared by all of that user's harnesses. */
export class SqliteCredentials implements CredentialStore {
  private readonly pending = new Map<string, Promise<unknown>>();
  constructor(private readonly db: ApplicationDatabase, private readonly userId: string, private readonly key: EncryptionKeySetting | undefined) {}
  private where(providerId: string) {
    return and(eq(hvProviderConnections.userId, this.userId), eq(hvProviderConnections.providerId, providerId));
  }
  requireKey() {
    if (!this.key) throw new AgentError("Provider connections require AGENTS_ENCRYPTION_KEY on the server.", 503);
    return this.key;
  }
  private serialize<T>(id: string, operation: () => Promise<T>): Promise<T> {
    const result = (this.pending.get(id) ?? Promise.resolve()).then(operation);
    const settled = result.then(() => {}, () => {});
    this.pending.set(id, settled);
    void settled.then(() => { if (this.pending.get(id) === settled) this.pending.delete(id); });
    return result;
  }
  async read(providerId: string, options?: AuthOperationOptions): Promise<Credential | undefined> {
    options?.signal?.throwIfAborted();
    const row = await this.db.select().from(hvProviderConnections).where(this.where(providerId)).get();
    if (!row) return undefined;
    const key = this.requireKey();
    if (row.keyId !== key.id) throw new Error("Provider encryption key is unavailable");
    const payload = JSON.parse(decryptSecret(key, row.ciphertext)) as { userId?: unknown; providerId?: unknown; credential?: Credential };
    if (payload.userId !== this.userId || payload.providerId !== providerId || payload.credential?.type !== row.authType) {
      throw new Error("Provider credential identity mismatch");
    }
    return payload.credential;
  }
  async list(options?: AuthOperationOptions) {
    options?.signal?.throwIfAborted();
    return this.db.select({ providerId: hvProviderConnections.providerId, type: hvProviderConnections.authType })
      .from(hvProviderConnections).where(eq(hvProviderConnections.userId, this.userId)).all();
  }
  modify(providerId: string, fn: (current: Credential | undefined) => Promise<Credential | undefined>, options?: AuthOperationOptions) {
    return this.serialize(providerId, async () => {
      options?.signal?.throwIfAborted();
      const current = await this.read(providerId, options);
      // OAuth refresh may use the network here. Hold only the per-provider
      // mutex, NEVER the shared SQLite queue/transaction across this callback.
      const next = await fn(current);
      if (next === undefined) return current;
      const key = this.requireKey();
      const values = { authType: next.type, ciphertext: Buffer.from(encryptSecret(key, JSON.stringify({ userId: this.userId, providerId, credential: next }))), keyId: key.id, updatedAt: Date.now() };
      await this.db.insert(hvProviderConnections).values({ userId: this.userId, providerId, ...values })
        .onConflictDoUpdate({ target: [hvProviderConnections.userId, hvProviderConnections.providerId], set: values }).run();
      return next;
    });
  }
  delete(providerId: string, options?: AuthOperationOptions) {
    return this.serialize(providerId, async () => {
      options?.signal?.throwIfAborted();
      await this.db.delete(hvProviderConnections).where(this.where(providerId)).run();
    });
  }
  catalogStore(): ModelsStore {
    return {
      read: async (id) => {
        const row = await this.db.select({ catalog: hvProviderConnections.catalog }).from(hvProviderConnections).where(this.where(id)).get();
        return row?.catalog ? JSON.parse(row.catalog) as ModelsStoreEntry : undefined;
      },
      write: async (id, entry) => {
        await this.db.update(hvProviderConnections).set({ catalog: JSON.stringify(entry) }).where(this.where(id)).run();
      },
      delete: async (id) => { await this.db.update(hvProviderConnections).set({ catalog: null }).where(this.where(id)).run(); },
    };
  }
  async drain() { await Promise.all(this.pending.values()); }
}
