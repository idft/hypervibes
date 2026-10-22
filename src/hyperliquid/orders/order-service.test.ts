import { eq } from "drizzle-orm";
import { HttpTransport } from "@nktkas/hyperliquid";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { openDatabase } from "../../db/database.js";
import { migrateApplication } from "../../db/migrations.js";
import { SharedDatabase, applicationDatabase } from "../../db/shared-database.js";
import { hvUsers } from "../../db/auth-schema.js";
import { hvAgents } from "../../db/agent-schema.js";
import { OrderService, type OrderBindings } from "./order-service.js";
import { HyperliquidError } from "../errors.js";
import type { OrderInput } from "./order-contracts.js";

const account = `0x${"ab".repeat(20)}`;
const owner = `0x${"cd".repeat(20)}`;
const privateKey = `0x${"12".repeat(32)}`;
const builder = { b: `0x${"ef".repeat(20)}` as `0x${string}`, f: 10 };
const order: OrderInput = { symbol: "BTC", side: "buy", order_type: "limit", size: "0.001", price: "100000" };
function remote(oid = 123, coin = "BTC", cloid?: string) {
  return { oid, coin, side: "B", sz: "0.001", origSz: "0.001", limitPx: "100000", orderType: "Limit", reduceOnly: false, isTrigger: false, triggerPx: "0", timestamp: Date.now(), ...(cloid ? { cloid } : {}) };
}
function receipt(statuses: unknown[], type = "order") { return { status: "ok", response: { type, data: { statuses } } }; }

describe("durable order gateway", () => {
  let directory: string, agentId: string;
  let database: SharedDatabase;
  let service: OrderService;
  let bindings: OrderBindings;
  let exchange: Mock<(body: Record<string, unknown>) => Promise<unknown>>;
  let status: Mock<(body: Record<string, unknown>) => Promise<unknown>>;
  let openOrders: Mock<(body: Record<string, unknown>) => Promise<unknown>>;
  let exchanges: Record<string, unknown>[];
  let infoCalls: Record<string, unknown>[];
  let positions: { coin: string; szi: string }[];
  const transport = () => new HttpTransport();
  const submit = (request_id = "request-1", orders = [order]) => service.submit("owner", agentId, { request_id, orders });
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-orders-"));
    const connection = openDatabase(join(directory, "app.sqlite")); migrateApplication(connection);
    database = new SharedDatabase(connection);
    agentId = randomUUID();
    const db = applicationDatabase(database);
    await db.insert(hvUsers).values({ id: "owner", name: "Owner", email: "owner@example.test" }).run();
    await db.insert(hvAgents).values({ id: agentId, userId: "owner", name: "Trader", state: "ready", tradingAccountAddress: account, creationRequestId: randomUUID(), createdAt: 1, updatedAt: 1 }).run();
    bindings = {
      account: vi.fn(async (user, agent) => { if (user !== "owner" || agent !== agentId) throw new HyperliquidError("Agent not found", 404); return account; }),
      authority: vi.fn(async () => ({ account, privateKey, vaultAddress: account as `0x${string}`, builder })),
    };
    exchanges = []; infoCalls = []; positions = [];
    exchange = vi.fn(async () => receipt([{ resting: { oid: 123 } }]));
    status = vi.fn(async () => ({ status: "unknownOid" }));
    openOrders = vi.fn(async () => []);
    vi.stubGlobal("fetch", vi.fn(async (_url: unknown, init: RequestInit) => {
      const body = JSON.parse(String(init.body)) as Record<string, unknown>;
      if (body.action) { exchanges.push(body); return Response.json(await exchange(body)); }
      infoCalls.push(body);
      if (body.type === "meta") return Response.json({ universe: [{ name: "BTC", szDecimals: 5, maxLeverage: 50 }, { name: "ETH", szDecimals: 4, maxLeverage: 25 }, { name: "DEAD", szDecimals: 2, isDelisted: true }] });
      if (body.type === "clearinghouseState") return Response.json({ assetPositions: positions.map((position) => ({ position })) });
      if (body.type === "allMids") return Response.json({ BTC: "100000", ETH: "2000" });
      if (body.type === "orderStatus") return Response.json(await status(body));
      if (body.type === "frontendOpenOrders") return Response.json(await openOrders(body));
      throw new Error(`Unexpected request ${body.type}`);
    }));
    service = new OrderService(database, transport(), bindings); await service.recover(); service.start();
  });
  afterEach(async () => { await service.close(); await database.close(); vi.unstubAllGlobals(); vi.restoreAllMocks(); vi.useRealTimers(); rmSync(directory, { recursive: true, force: true }); });

  it("blocks every model mutation while disabled, including prepared retries, but keeps reads available", async () => {
    vi.spyOn(service.store, "begin").mockRejectedValueOnce(new Error("before send"));
    await expect(submit("prepared")).rejects.toThrow("before send");
    await applicationDatabase(database).update(hvAgents).set({ enabled: false }).where(eq(hvAgents.id, agentId)).run();
    await expect(submit("prepared")).rejects.toMatchObject({ status: 409 });
    await expect(submit("reduction", [{ ...order, reduce_only: true }])).rejects.toMatchObject({ status: 409 });
    await expect(service.cancel("owner", agentId, { request_id: "cancel", orders: [{ symbol: "BTC", oid: "123" }] })).rejects.toMatchObject({ status: 409 });
    await expect(service.cancelAll("owner", agentId, { request_id: "cancel-all" })).rejects.toMatchObject({ status: 409 });
    expect(await service.request("owner", agentId, "prepared")).toBeDefined();
    expect(exchange).not.toHaveBeenCalled();
  });

  it.each(["0.002", "-0.002"])("closes live signed size %s reduce-only while disabled and never resubmits an old operation", async (szi) => {
    positions = [{ coin: "BTC", szi }];
    await applicationDatabase(database).update(hvAgents).set({ enabled: false }).where(eq(hvAgents.id, agentId)).run();
    exchange.mockImplementation(async () => {
      positions = [];
      return receipt([{ filled: { oid: 123, totalSz: "0.002", avgPx: "100000" } }]);
    });
    const result = await service.closePosition("owner", agentId, "BTC", "manual-close");
    expect(result.complete).toBe(true);
    expect(exchanges[0]).toMatchObject({ action: { type: "order", orders: [{ b: szi.startsWith("-"), r: true, s: "0.002", t: { limit: { tif: "Ioc" } } }] } });
    positions = [{ coin: "BTC", szi }];
    expect((await service.closePosition("owner", agentId, "BTC", "manual-close")).complete).toBe(false);
    expect(exchange).toHaveBeenCalledOnce();
    await expect(service.closePosition("other", agentId, "BTC", "wrong-owner")).rejects.toMatchObject({ status: 404 });
    await expect(service.closePosition("owner", agentId, "ETH", "manual-close")).rejects.toMatchObject({ status: 409 });
  });

  it("cancels only the selected market's exchange orders before closing and reports known partial fills", async () => {
    positions = [{ coin: "BTC", szi: "0.002" }];
    let orders = [remote(), remote(456, "ETH")];
    openOrders.mockImplementation(async () => orders);
    status.mockImplementation(async () => ({ status: "order", order: { order: remote(), status: "open", statusTimestamp: 1 } }));
    exchange.mockImplementation(async (body) => {
      if ((body.action as { type: string }).type === "cancel") {
        orders = [remote(456, "ETH")];
        return receipt(["success"], "cancel");
      }
      positions = [{ coin: "BTC", szi: "0.001" }];
      return receipt([{ filled: { oid: 789, totalSz: "0.001", avgPx: "100000" } }]);
    });
    const result = await service.closePosition("owner", agentId, "BTC", "partial-close");
    expect(exchanges[0]).toMatchObject({ action: { type: "cancel", cancels: [{ a: 0, o: 123 }] } });
    expect(exchanges[1]).toMatchObject({ action: { type: "order", orders: [{ r: true, s: "0.002" }] } });
    expect(result).toMatchObject({ complete: false, remainingSize: "0.001", remainingOrders: 0 });
    expect(result.warnings.join(" ")).toContain("part");
    expect(orders[0]?.coin).toBe("ETH");
  });

  it("preserves unknown closes across retries and refuses a blind new close ID", async () => {
    positions = [{ coin: "BTC", szi: "0.002" }];
    exchange.mockRejectedValue(new Error("lost response"));
    expect((await service.closePosition("owner", agentId, "BTC", "unknown-close")).close.uncertain).toBe(true);
    expect((await service.closePosition("owner", agentId, "BTC", "unknown-close")).close.uncertain).toBe(true);
    await expect(service.closePosition("owner", agentId, "BTC", "blind-retry")).rejects.toThrow("unknown outcome");
    expect(exchange).toHaveBeenCalledOnce();
  });

  it("joins already-started submissions when disabling without canceling them", async () => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => { release = resolve; });
    exchange.mockImplementationOnce(async () => { await held; return receipt([{ resting: { oid: 123 } }]); });
    const submitting = submit();
    await vi.waitFor(() => expect(exchange).toHaveBeenCalledOnce());
    await applicationDatabase(database).update(hvAgents).set({ enabled: false }).where(eq(hvAgents.id, agentId)).run();
    let joined = false;
    const joining = service.quiesce(agentId).then(() => { joined = true; });
    const blocked = expect(submit("blocked")).rejects.toMatchObject({ status: 409 });
    // The blocked command is behind the in-flight submission too.
    expect(joined).toBe(false);
    release(); await submitting; await joining; await blocked;
    expect(exchange).toHaveBeenCalledOnce();
  });

  it("commits intents before signed transport and deduplicates concurrent requests and restarts", async () => {
    exchange.mockImplementation(async (body) => {
      expect(await database.get("SELECT id FROM hv_order_actions WHERE state='submitting' AND nonce=?", body.nonce as number)).toBeDefined();
      expect(body).toMatchObject({ vaultAddress: account, action: { type: "order", grouping: "na", builder } });
      expect(body.signature).toMatchObject({ r: expect.stringMatching(/^0x/), s: expect.stringMatching(/^0x/), v: expect.any(Number) });
      return receipt([{ resting: { oid: 123 } }]);
    });
    const [first, second] = await Promise.all([submit(), submit()]);
    expect(first).toEqual(second); expect(exchange).toHaveBeenCalledOnce();
    expect(first.results[0]).toMatchObject({ commandStatus: "accepted", orderStatus: "open", exchangeOid: "123", size: "0.001", price: "100000" });
    expect(first.results[0]!.cloid).toMatch(/^0x[0-9a-f]{32}$/);
    const action = (exchanges[0]!.action as { orders: { c: string; p: string; s: string }[] }).orders[0]!;
    expect(action).toMatchObject({ c: first.results[0]!.cloid, p: "100000", s: "0.001" });
    await expect(submit("request-1", [{ ...order, size: "0.002" }])).rejects.toMatchObject({ status: 409 });
    await service.close(); service = new OrderService(database, transport(), bindings); await service.recover();
    expect(exchange).toHaveBeenCalledOnce(); service.start();
    expect((await submit()).results[0]!.cloid).toBe(first.results[0]!.cloid);
    expect(exchange).toHaveBeenCalledOnce();
    expect(JSON.stringify(await database.all("SELECT payload FROM hv_order_actions"))).not.toContain(privateKey);
  });

  it("seals per-agent order admission and joins an in-flight exchange submission before deletion", async () => {
    let release!: () => void;
    const held = new Promise<void>((resolve) => { release = resolve; });
    exchange.mockImplementationOnce(async () => { await held; return receipt([{ resting: { oid: 123 } }]); });
    const submitting = submit();
    await vi.waitFor(() => expect(exchange).toHaveBeenCalledOnce());
    let stopped = false;
    const stopping = service.stopAgent(agentId).then(() => { stopped = true; });
    try {
      await expect(submit("new-request")).rejects.toThrow("being deleted");
      await expect(service.request("owner", agentId, "request-1")).rejects.toThrow("being deleted");
      expect(stopped).toBe(false);
    } finally { release(); }
    expect((await submitting).results[0]!.commandStatus).toBe("accepted");
    await stopping;
    expect(stopped).toBe(true);
    expect(exchange).toHaveBeenCalledOnce();
  });

  it("keeps mixed successes and rejections, including native grouped TP/SL", async () => {
    exchange.mockResolvedValue(receipt([{ resting: { oid: 123 } }, "waitingForFill", { error: "bad trigger" }]));
    const result = await submit("protected", [{ ...order, take_profits: [{ trigger_price: "110000" }], stop_losses: [{ trigger_price: "90000" }] }]);
    expect(result.results.map((item) => item.commandStatus)).toEqual(["accepted", "accepted", "rejected"]);
    expect(result.results[2]!.error).toBe("bad trigger");
    expect(exchanges[0]).toMatchObject({ action: { grouping: "normalTpsl", orders: [
      { b: true, r: false, s: "0.001" },
      { b: false, r: true, p: "99000", t: { trigger: { triggerPx: "110000", isMarket: true, tpsl: "tp" } } },
      { b: false, r: true, p: "81000", t: { trigger: { triggerPx: "90000", isMarket: true, tpsl: "sl" } } },
    ] } });
    expect(result.warnings.join(" ")).toContain("unprotected");
  });

  it("uses exact IOC guards and reports actual partial fill quantity", async () => {
    exchange.mockResolvedValue(receipt([{ filled: { oid: 123, totalSz: "0.0005", avgPx: "100001.12345" } }]));
    const result = await submit("market", [{ symbol: "BTC", side: "buy", order_type: "market", size: "0.001" }]);
    expect(exchanges[0]).toMatchObject({ action: { orders: [{ p: "100500", s: "0.001", t: { limit: { tif: "Ioc" } } }] } });
    expect(result.results[0]).toMatchObject({ filledSize: "0.0005", averageFillPrice: "100001.12345" });
  });

  it("preserves unknown submission outcomes, blocks blind new exposure, and resolves by persisted CLOID", async () => {
    exchange.mockRejectedValue(new Error("transport lost after venue acceptance"));
    const result = await submit();
    expect(result).toMatchObject({ uncertain: true, results: [{ commandStatus: "unknown" }] });
    expect(await submit()).toEqual(result);
    await expect(submit("different-id")).rejects.toThrow("unknown outcome");
    const unknown = await service.request("owner", agentId, "request-1");
    expect(unknown.uncertain).toBe(true);
    expect(unknown.results[0]!.error).toContain("absence is not proof");
    const cloid = result.results[0]!.cloid!;
    status.mockImplementation(async (body) => { expect(body).toMatchObject({ user: account, oid: cloid }); return { status: "order", order: { order: remote(123, "BTC", cloid), status: "open", statusTimestamp: Date.now() } }; });
    const reconciled = await service.request("owner", agentId, "request-1");
    expect(reconciled).toMatchObject({ uncertain: false, results: [{ commandStatus: "reconciled", exchangeOid: "123", orderStatus: "open" }] });
    expect(exchange).toHaveBeenCalledOnce();
  });

  it("recovers the crash-after-send-before-outcome window without resubmission", async () => {
    vi.spyOn(service.store, "outcome").mockRejectedValueOnce(new Error("outcome persistence failure"));
    await expect(submit()).rejects.toThrow("outcome persistence failure");
    expect(exchange).toHaveBeenCalledOnce();
    await service.close(); service = new OrderService(database, transport(), bindings); await service.recover(); service.start();
    const result = await submit();
    expect(result).toMatchObject({ uncertain: true, results: [{ commandStatus: "unknown" }] });
    expect(exchange).toHaveBeenCalledOnce();
  });

  it("never submits an expired prepared intent or new groups after a prior unknown group", async () => {
    exchange.mockRejectedValueOnce(new Error("offline"));
    const result = await submit("batch", [order, { ...order, symbol: "ETH", size: "0.01", price: "2000" }]);
    expect(result.results.map((item) => item.commandStatus)).toEqual(["unknown", "not_sent"]);
    expect(exchange).toHaveBeenCalledOnce();
    expect((await submit("batch", [order, { ...order, symbol: "ETH", size: "0.01", price: "2000" }])).results[1]!.commandStatus).toBe("not_sent");
    expect(exchange).toHaveBeenCalledOnce();
  });

  it("authorizes cancellation against account/symbol and captures cancel-all exactly once", async () => {
    openOrders.mockResolvedValue([remote(123), remote(999, "@107"), remote(998, "PURR/USDC")]);
    status.mockResolvedValue({ status: "order", order: { order: remote(), status: "open", statusTimestamp: Date.now() } });
    exchange.mockResolvedValue(receipt(["success"], "cancel"));
    const input = { request_id: "cancel-snapshot" };
    const result = await service.cancelAll("owner", agentId, input);
    expect(result.results[0]).toMatchObject({ kind: "cancel", exchangeOid: "123", commandStatus: "accepted", orderStatus: "canceled" });
    expect(exchanges[0]).toMatchObject({ vaultAddress: account, action: { type: "cancel", cancels: [{ a: 0, o: 123 }] } });
    openOrders.mockResolvedValue([remote(456)]);
    expect(await service.cancelAll("owner", agentId, input)).toEqual(result);
    expect(openOrders).toHaveBeenCalledOnce(); expect(exchange).toHaveBeenCalledOnce();
    await expect(service.cancel("owner", agentId, { request_id: "bad-symbol", orders: [{ symbol: "ETH", oid: "123" }] })).rejects.toThrow("not found");
    expect(exchange).toHaveBeenCalledOnce();
  });

  it("does not report cancellation success from absence or a still-open report", async () => {
    status.mockResolvedValue({ status: "order", order: { order: remote(), status: "open", statusTimestamp: Date.now() } });
    exchange.mockRejectedValue(new Error("lost cancel response"));
    await service.cancel("owner", agentId, { request_id: "cancel", orders: [{ symbol: "BTC", oid: "123" }] });
    expect((await service.request("owner", agentId, "cancel")).uncertain).toBe(true);
    status.mockResolvedValue({ status: "unknownOid" });
    expect((await service.request("owner", agentId, "cancel")).uncertain).toBe(true);
    status.mockResolvedValue({ status: "order", order: { order: remote(), status: "filled", statusTimestamp: Date.now() } });
    const result = await service.request("owner", agentId, "cancel");
    expect(result).toMatchObject({ uncertain: false, results: [{ commandStatus: "reconciled", orderStatus: "filled" }] });
    expect(exchange).toHaveBeenCalledOnce();
  });

  it("enforces account isolation for every path before exchange lookup", async () => {
    for (const operation of [
      () => service.submit("other", agentId, { request_id: "x", orders: [order] }),
      () => service.cancel("other", agentId, { request_id: "x", orders: [{ symbol: "BTC", oid: "123" }] }),
      () => service.cancelAll("other", agentId, { request_id: "x" }),
      () => service.list("other", agentId, {}), () => service.get("other", agentId, "123"), () => service.request("other", agentId, "x"),
    ]) await expect(operation()).rejects.toMatchObject({ status: 404 });
    expect(infoCalls).toEqual([]); expect(exchange).not.toHaveBeenCalled();
    expect(bindings.authority).not.toHaveBeenCalled();
  });

  it("rejects invalid precision, symbols, attachments, and numeric money without submitting", async () => {
    for (const value of [
      { ...order, size: "0.000001" }, { ...order, price: "100000.1" }, { ...order, symbol: "@107" }, { ...order, symbol: "DEAD" },
      { ...order, take_profits: [{ trigger_price: "90000" }] }, { ...order, reduce_only: true, stop_losses: [{ trigger_price: "90000" }] },
      { ...order, stop_losses: [{ trigger_price: "90000", limit_price: "80000" }] },
    ]) await expect(submit(randomUUID(), [value])).rejects.toThrow();
    expect(() => service.submit("owner", agentId, { request_id: "numeric", orders: [{ ...order, size: 0.1 as unknown as string }] })).toThrow("decimal string");
    expect(exchange).not.toHaveBeenCalled();
    expect(await database.all("SELECT * FROM hv_order_requests")).toEqual([]);
  });

  it("reads external open orders, refreshes local order status, and bounds events", async () => {
    openOrders.mockResolvedValue([remote(), remote(456, "@107"), remote(789, "PURR/USDC")]);
    const listed = await service.list("owner", agentId, {});
    expect(listed.items).toHaveLength(1); expect(listed.items[0]!.exchangeOid).toBe("123");
    const placed = await submit();
    status.mockResolvedValue({ status: "order", order: { order: remote(123, "BTC", placed.results[0]!.cloid!), status: "canceled", statusTimestamp: Date.now() } });
    const got = await service.get("owner", agentId, placed.results[0]!.id, true);
    expect(got.order).toMatchObject({ id: placed.results[0]!.id, orderStatus: "canceled" });
    expect(got.events.map((event) => event.source)).toContain("reconciliation");
    expect((await service.get("owner", agentId, placed.results[0]!.cloid!)).order?.id).toBe(placed.results[0]!.id);
    expect((await service.list("owner", agentId, { source: "local" })).items[0]!.orderStatus).toBe("canceled");
  });

  it("records the venue's $10 minimum rejection without increasing the requested size", async () => {
    exchange.mockResolvedValue(receipt([{ error: "Order must have minimum value of $10." }]));
    const result = await submit("too-small", [{ ...order, size: "0.00001" }]);
    expect(result.results[0]).toMatchObject({ size: "0.00001", commandStatus: "rejected", orderStatus: "rejected", error: "Order must have minimum value of $10." });
    expect(exchanges[0]).toMatchObject({ action: { orders: [{ s: "0.00001" }] } });
    expect(await submit("too-small", [{ ...order, size: "0.00001" }])).toEqual(result);
    expect(exchange).toHaveBeenCalledOnce();
  });

  it("does not send a persisted prepared request after its expiration", async () => {
    const begin = vi.spyOn(service.store, "begin").mockResolvedValueOnce(false);
    const prepared = await submit("expired");
    expect(prepared.results[0]!.commandStatus).toBe("not_sent");
    begin.mockRestore();
    await database.run("UPDATE hv_order_actions SET expires_at = ?", Date.now() - 1);
    await service.close(); service = new OrderService(database, transport(), bindings); await service.recover(); service.start();
    const result = await submit("expired");
    expect(result.results[0]).toMatchObject({ commandStatus: "not_sent", orderStatus: "not_sent" });
    expect(result.results[0]!.error).toContain("expired");
    expect(exchange).not.toHaveBeenCalled();
  });

  it("retains uncertainty on malformed exchange receipts instead of inventing success", async () => {
    exchange.mockResolvedValue(receipt([]));
    const result = await submit();
    expect(result).toMatchObject({ uncertain: true, results: [{ commandStatus: "unknown" }] });
    expect(result.results[0]!.error).toContain("shape/count");
  });

  it("does not regress venue-timestamped state or duplicate identical reconciliation events", async () => {
    const placed = await submit();
    const report = remote(123, "BTC", placed.results[0]!.cloid!);
    status.mockResolvedValue({ status: "order", order: { order: report, status: "open", statusTimestamp: 200 } });
    await service.request("owner", agentId, "request-1");
    const events = await service.store.events(placed.results[0]!.id);
    await service.request("owner", agentId, "request-1");
    expect(await service.store.events(placed.results[0]!.id)).toHaveLength(events.length);
    status.mockResolvedValue({ status: "order", order: { order: report, status: "canceled", statusTimestamp: 100 } });
    expect((await service.request("owner", agentId, "request-1")).results[0]).toMatchObject({ orderStatus: "open", venueTimestamp: 200 });
  });

  it("keeps previous account assignments out of local reads and refuses replay on a changed assignment", async () => {
    await submit();
    bindings.account = vi.fn(async () => owner);
    expect((await service.list("owner", agentId, { source: "local" })).items).toEqual([]);
    await expect(submit()).rejects.toMatchObject({ status: 409 });
    await expect(service.request("owner", agentId, "request-1")).rejects.toMatchObject({ status: 404 });
    expect(exchange).toHaveBeenCalledOnce();
  });

  it("uses monotonic persisted per-signer nonces and omits vaultAddress for main accounts", async () => {
    bindings.authority = vi.fn(async () => ({ account, privateKey, builder }));
    await submit("one"); await submit("two");
    expect(Number(exchanges[1]!.nonce)).toBeGreaterThan(Number(exchanges[0]!.nonce));
    expect(exchanges[0]).not.toHaveProperty("vaultAddress");
    expect((await database.all("SELECT * FROM hv_exchange_nonces"))).toHaveLength(1);
    expect(owner).not.toBe(account);
  });

  it("does not run reconciliation on open, but reconciles pending state after start", async () => {
    exchange.mockRejectedValue(new Error("lost")); await submit();
    await service.close();
    status.mockClear();
    service = new OrderService(database, transport(), bindings); await service.recover();
    expect(status).not.toHaveBeenCalled();
    service.start();
    await vi.waitFor(() => expect(status).toHaveBeenCalled());
    expect(exchange).toHaveBeenCalledOnce();
  });
});
