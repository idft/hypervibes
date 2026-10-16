import { drizzle } from "drizzle-orm/sqlite-proxy";
import type { DatabaseSync, SQLInputValue, SQLOutputValue } from "node:sqlite";
import type { SqliteDatabase, SqliteExecutor, SqliteValue } from "@earendil-works/pi-durable/storage/sqlite";

type Method = "run" | "all" | "get" | "values";

class Executor implements SqliteExecutor {
  active = true;
  constructor(private readonly connection: DatabaseSync) {}
  private statement(sql: string) {
    if (!this.active) throw new Error("SQLite executor is closed");
    return this.connection.prepare(sql);
  }
  async exec(sql: string) {
    if (!this.active) throw new Error("SQLite executor is closed");
    this.connection.exec(sql);
  }
  async run(sql: string, ...params: SqliteValue[]) { this.statement(sql).run(...params); }
  async get<T extends object>(sql: string, ...params: SqliteValue[]) {
    return this.statement(sql).get(...params) as T | undefined;
  }
  async all<T extends object>(sql: string, ...params: SqliteValue[]) {
    return this.statement(sql).all(...params) as T[];
  }
  async query(sql: string, params: SQLInputValue[], method: Method) {
    // Transactions must reserve the queue through SharedDatabase.transaction(),
    // never through the proxy driver's separate BEGIN/query/COMMIT operations.
    if (/^\s*(BEGIN|COMMIT|ROLLBACK|SAVEPOINT|RELEASE)\b/i.test(sql)) {
      throw new Error("Use the shared database transaction boundary");
    }
    const statement = this.statement(sql);
    if (method === "run") { statement.run(...params); return { rows: [] }; }
    statement.setReturnArrays(true);
    // node:sqlite's declarations retain object return types after this setting.
    // Drizzle's callback declares rows as an array even for `get`, but its
    // runtime requires undefined for a missing row (an empty array maps to a
    // phantom record). Preserve undefined through this driver type boundary.
    if (method === "get") return { rows: statement.get(...params) as unknown as SQLOutputValue[] };
    return { rows: statement.all(...params) as unknown as SQLOutputValue[][] };
  }
}

/** One queue for app SQL and every harness, held across the entire transaction. */
export class SharedDatabase implements SqliteDatabase {
  private tail: Promise<unknown> = Promise.resolve();
  private closed = false;
  private failure?: Error;
  private readonly executor: Executor;
  constructor(private readonly connection: DatabaseSync) { this.executor = new Executor(connection); }
  get available() { return !this.closed && !this.failure; }
  private enqueue<T>(operation: () => Promise<T>): Promise<T> {
    if (this.closed) return Promise.reject(new Error("Database is closed"));
    if (this.failure) return Promise.reject(this.failure);
    const result = this.tail.then(() => {
      if (this.failure) throw this.failure;
      return operation();
    });
    this.tail = result.catch(() => {});
    return result;
  }
  exec(sql: string) { return this.enqueue(() => this.executor.exec(sql)); }
  run(sql: string, ...params: SqliteValue[]) { return this.enqueue(() => this.executor.run(sql, ...params)); }
  get<T extends object>(sql: string, ...params: SqliteValue[]) { return this.enqueue(() => this.executor.get<T>(sql, ...params)); }
  all<T extends object>(sql: string, ...params: SqliteValue[]) { return this.enqueue(() => this.executor.all<T>(sql, ...params)); }
  query(sql: string, params: SQLInputValue[], method: Method) {
    return this.enqueue(() => this.executor.query(sql, params, method));
  }
  transaction<T>(callback: (transaction: SqliteExecutor) => Promise<T>): Promise<T> {
    return this.enqueue(async () => {
      const transaction = new Executor(this.connection);
      this.connection.exec("BEGIN IMMEDIATE");
      try {
        const result = await callback(transaction);
        this.connection.exec("COMMIT");
        return result;
      } catch (error) {
        try { this.connection.exec("ROLLBACK"); }
        catch (rollback) {
          // Transaction ownership is now uncertain. Never let another owner
          // accidentally join it; shutdown may still close the raw connection.
          this.failure = new AggregateError([error, rollback], "SQLite rollback failed", { cause: rollback });
          throw this.failure;
        }
        throw error;
      } finally { transaction.active = false; }
    });
  }
  appTransaction<T>(callback: (db: ApplicationDatabase) => Promise<T>) {
    return this.transaction((transaction) => callback(drizzle((sql, params, method) =>
      (transaction as Executor).query(sql, params, method))));
  }
  /** Each harness borrows the facade; only the application owns the connection. */
  borrow(): SqliteDatabase {
    return {
      exec: this.exec.bind(this), run: this.run.bind(this), get: this.get.bind(this), all: this.all.bind(this),
      transaction: this.transaction.bind(this),
      // Storage close drains already-admitted SQL without closing other owners.
      close: () => this.enqueue(async () => {}),
    };
  }
  async close() {
    if (this.closed) return this.tail.then(() => {});
    this.closed = true;
    await this.tail;
    this.executor.active = false;
    if (this.connection.isOpen) this.connection.close();
  }
}

export function applicationDatabase(database: SharedDatabase) {
  return drizzle((sql, params, method) => database.query(sql, params, method));
}
export type ApplicationDatabase = ReturnType<typeof applicationDatabase>;
