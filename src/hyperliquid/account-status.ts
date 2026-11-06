import { isDeepStrictEqual } from "node:util";
import { InfoClient, type HttpTransport } from "@nktkas/hyperliquid";
import { Type, type Static, type TSchema } from "typebox";
import { Check } from "typebox/value";
import {
  AccountModeSchema, AccountOrderSchema, AccountPerpStateSchema, AccountSpotStateSchema,
  type AccountStatus,
} from "./account-status-schema.js";
export type { AccountStatus } from "./account-status-schema.js";

type Section = AccountStatus["sections"]["spot"];
type Order = Static<typeof AccountOrderSchema>;
const unavailable = (): Section => ({ updatedAt: null, fresh: false, source: "unavailable" });
const received = (): Section => ({ updatedAt: Date.now(), fresh: true, source: "http" });
const discoverySchema = Type.Array(Type.Union([
  Type.Null(), Type.Object({ name: Type.String({ minLength: 1, maxLength: 64 }) }),
]), { minItems: 1, maxItems: 256 });

function validate<T extends TSchema>(schema: T, value: unknown): Static<T> {
  if (!Check(schema, value)) throw new Error("Invalid account response");
  return value;
}

function isSpot(coin: string) { return /^(@[0-9]+|[^\s:/@#+]+\/[^\s:/@#+]+)(?![\s\S])/.test(coin); }
// Official asset-ids docs distinguish outcome order coins (#N) from balance names (+N/oN).
function isOutcome(coin: string) { return /^#[0-9]+(?![\s\S])/.test(coin); }
function matchesPerpDex(coin: string, dex: string) {
  if (/[\s/@#+]/.test(coin) || /^o[0-9]+(?![\s\S])/.test(coin)) return false;
  const parts = coin.split(":");
  return dex === "" ? parts.length === 1 : parts.length === 2 && parts[0] === dex && Boolean(parts[1]);
}

/** Preserve child order identity without recursive output schemas or double counting parents. */
function flattenOrders(value: unknown): Order[] {
  if (!Array.isArray(value)) throw new Error("Invalid order list");
  const output: Order[] = [];
  const knownIds = new Map<number, { order: Order; fields: Record<string, unknown> }>();
  let count = 0;
  const visit = (items: unknown[], parentOid: number | null, path: number[], ancestorIds: number[]) => {
    if (path.length > 16) throw new Error("Excessive order nesting");
    for (const [index, item] of items.entries()) {
      if (!item || typeof item !== "object" || Array.isArray(item)) throw new Error("Invalid order");
      const raw = item as Record<string, unknown>;
      const { children, ...fields } = raw;
      const sourcePath = [...path, index];
      const order = validate(AccountOrderSchema, { ...fields, parentOid, sourcePath, orderIdKnown: typeof fields.oid === "number" && fields.oid > 0 });
      if (++count > 10_000) throw new Error("Excessive order count");
      if (order.orderIdKnown && ancestorIds.includes(order.oid)) throw new Error("Cyclic order identity");
      const existing = order.orderIdKnown ? knownIds.get(order.oid) : undefined;
      if (existing) {
        if (!isDeepStrictEqual(existing.fields, fields)) throw new Error("Conflicting duplicate order ID");
        if (existing.order.parentOid !== null && parentOid !== null && existing.order.parentOid !== parentOid) throw new Error("Conflicting order parents");
        if (parentOid !== null) {
          existing.order.parentOid = parentOid;
          existing.order.sourcePath = sourcePath;
        }
      } else {
        output.push(order);
        if (order.orderIdKnown) knownIds.set(order.oid, { order, fields });
      }
      if (children !== undefined) {
        if (!Array.isArray(children)) throw new Error("Invalid child orders");
        visit(children, order.oid, sourcePath, [...ancestorIds, ...(order.orderIdKnown ? [order.oid] : [])]);
      }
    }
  };
  visit(value, null, [], []);
  return output;
}

/** Enforce the deadline even if an injected transport does not handle its signal. */
async function bounded<T>(request: () => Promise<T>, signal: AbortSignal): Promise<T> {
  signal.throwIfAborted();
  let abort: (() => void) | undefined;
  const aborted = new Promise<never>((_resolve, reject) => {
    abort = () => reject(new Error("Account status request aborted or timed out"));
    signal.addEventListener("abort", abort, { once: true });
  });
  try { return await Promise.race([request(), aborted]); }
  finally { if (abort) signal.removeEventListener("abort", abort); }
}

/**
 * Fresh, read-only HTTP snapshot. Never use dashboard caches as account coverage.
 * Official API docs: info-endpoint (frontendOpenOrders includes spot only on dex ""),
 * info-endpoint/perpetuals (perpDexs null = main DEX), and info-endpoint/spot.
 * Under unified/portfolio margin, spot balances are the balance source of truth.
 * Per-DEX summaries and withdrawable are NOT account equity or available margin.
 */
export async function loadAccountStatus(transport: HttpTransport, address: string | null, cancellation?: AbortSignal): Promise<AccountStatus> {
  if (cancellation?.aborted) throw new Error("Account status request aborted");
  const result: AccountStatus = {
    address, scope: "account-wide-perpetuals-and-spot", startedAt: Date.now(), observedAt: Date.now(), accountAbstraction: null,
    equity: { value: null, reason: "Account-wide equity is unknown. No verified valuation and aggregation rule across spot, perpetual DEXs, and account modes is applied. Do not sum spot balances with perpetual accountValue." },
    availableMargin: { value: null, reason: "Account-wide available margin is unknown. Per-DEX withdrawable, raw margin summaries, and spot total minus hold are not verified trading capacity." },
    coverage: { complete: false, openingRiskVerified: false, perpDexs: null, discovery: unavailable(), limitations: [
      "Only this address: discovered perpetual DEXs, spot balances, and frontend open orders. No subaccounts, vault equity, staking, HyperEVM balances, or pending TWAP schedules.",
      "HTTP reads are not atomic. updatedAt records local receipt; perpetual state.time records exchange time. Fresh perpetual state must be at most 30 seconds old.",
      "Per-DEX summaries retain exchange units and are not added together. Unified and portfolio-margin trading balances come from spot state.",
      "Order children are flattened with parentOid. Identical positive order IDs are deduplicated, preserving parent linkage. Zero IDs are placeholders: orderIdKnown is false and sourcePath identifies their response location.",
      "Outcome order coins (#N) are separate; outcome balances (+N/oN), if returned by the spot endpoint, remain in spot state.",
      "coverage.complete describes only requested HTTP source coverage, not sufficient account-wide opening-risk verification. Pending TWAP schedules are not queried.",
    ] },
    perps: [], spot: { state: null, orders: null, sections: { balances: unavailable(), orders: unavailable() } },
    outcomeOrders: { orders: null, section: unavailable() },
    perpEquity: null, positions: null, spotUsdc: null, spotUsdcHold: null, orders: null,
    sections: { perps: unavailable(), spot: unavailable(), orders: unavailable(), mode: unavailable() }, errors: [],
  };
  if (!address) { result.errors.push("No trading account is assigned to this Agent."); return result; }
  if (!/^0x[a-fA-F0-9]{40}(?![\s\S])/.test(address)) { result.errors.push("The assigned trading account address is invalid."); return result; }
  const client = new InfoClient({ transport });
  const user = address as `0x${string}`;
  const signal = AbortSignal.any([AbortSignal.timeout(8_000), ...(cancellation ? [cancellation] : [])]);
  const attempt = async (label: string, action: () => Promise<void>) => {
    try { signal.throwIfAborted(); await action(); }
    catch {
      if (cancellation?.aborted) throw new Error("Account status request aborted");
      result.errors.push(`${label} unavailable: request failed, timed out, or returned malformed data.`);
    }
  };
  // This phase has at most 3 requests. The per-DEX phase below has at most 4.
  await Promise.all([
    attempt("Perpetual DEX discovery", async () => {
      const dexs = validate(discoverySchema, await bounded(() => client.perpDexs(signal), signal));
      if (dexs[0] !== null || dexs.slice(1).some((dex) => dex === null)) throw new Error("Invalid main DEX marker");
      const names = dexs.map((dex) => dex?.name ?? "");
      if (new Set(names).size !== names.length) throw new Error("Duplicate DEX names");
      result.coverage.perpDexs = names;
      result.coverage.discovery = received();
    }),
    attempt("Spot balances and holds", async () => {
      const state = validate(AccountSpotStateSchema, await bounded(() => client.spotClearinghouseState({ user }, signal), signal));
      const identities = state.balances.map((balance) => "token" in balance ? `token:${balance.token}` : `outcome:${balance.coin}`);
      if (new Set(identities).size !== identities.length) throw new Error("Duplicate spot token identity");
      if (state.balances.some((balance) => ("token" in balance && balance.token === 0) !== (balance.coin === "USDC"))) throw new Error("Inconsistent USDC token identity");
      result.spot.state = state;
      const usdc = state.balances.find((balance) => "token" in balance && balance.token === 0 && balance.coin === "USDC");
      result.spotUsdc = usdc?.total ?? "0";
      result.spotUsdcHold = usdc?.hold ?? "0";
      result.sections.spot = received();
      result.spot.sections.balances = result.sections.spot;
    }),
    attempt("Account mode", async () => {
      result.accountAbstraction = validate(AccountModeSchema, await bounded(() => client.userAbstraction({ user }, signal), signal));
      result.sections.mode = received();
    }),
  ]);
  // Discovery failure is not an empty account. Main-DEX reads can still succeed,
  // but coverage.perpDexs stays null and coverage.complete stays false.
  result.perps = (result.coverage.perpDexs ?? [""]).map((dex) => ({ dex, state: null, orders: null, sections: { state: unavailable(), orders: unavailable() } }));
  const jobs = result.perps.flatMap((perp) => [
    () => attempt(`Perpetual state (${perp.dex || "main"})`, async () => {
      const state = validate(AccountPerpStateSchema, await bounded(() => client.clearinghouseState({ user, dex: perp.dex }, signal), signal));
      if (state.assetPositions.some(({ position }) => !matchesPerpDex(position.coin, perp.dex))) throw new Error("Position DEX identity mismatch");
      perp.state = state;
      perp.sections.state = received();
      if (perp.dex === "") {
        result.perpEquity = state.marginSummary.accountValue;
        result.positions = state.assetPositions.map(({ position: p }) => ({ coin: p.coin, size: p.szi, entryPrice: p.entryPx, unrealizedPnl: p.unrealizedPnl, liquidationPrice: p.liquidationPx, leverage: p.leverage.value }));
        result.sections.perps = perp.sections.state;
      }
    }),
    () => attempt(`Open orders (${perp.dex || "main"})`, async () => {
      const orders = flattenOrders(await bounded(() => client.frontendOpenOrders({ user, dex: perp.dex }, signal), signal));
      if (orders.some((order) => !matchesPerpDex(order.coin, perp.dex) && !(perp.dex === "" && (isSpot(order.coin) || isOutcome(order.coin))))) throw new Error("Order DEX identity mismatch");
      perp.orders = orders.filter((order) => !isSpot(order.coin) && !isOutcome(order.coin));
      perp.sections.orders = received();
      if (perp.dex === "") {
        result.spot.orders = orders.filter((order) => isSpot(order.coin));
        result.spot.sections.orders = perp.sections.orders;
        result.outcomeOrders = { orders: orders.filter((order) => isOutcome(order.coin)), section: perp.sections.orders };
        result.orders = perp.orders.map((order) => ({ id: order.oid, coin: order.coin, side: order.side === "B" ? "Buy" : "Sell", size: order.sz, price: order.limitPx, type: order.orderType,
          timestamp: order.timestamp, ...(order.isTrigger ? { triggerPrice: order.triggerPx } : {}), ...(order.isPositionTpsl ? { positionTpsl: true as const } : {}),
        }));
        result.sections.orders = perp.sections.orders;
      }
    }),
  ]);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(4, jobs.length) }, async () => {
    while (next < jobs.length) { const job = jobs[next++]; if (job) await job(); }
  }));
  if (cancellation?.aborted) throw new Error("Account status request aborted");
  result.observedAt = Date.now();
  const sections = [result.coverage.discovery, ...Object.values(result.sections), ...Object.values(result.spot.sections),
    result.outcomeOrders.section, ...result.perps.flatMap((perp) => Object.values(perp.sections))];
  for (const section of sections) {
    if (section.updatedAt !== null) {
      const age = result.observedAt - section.updatedAt;
      section.fresh = section.fresh && age >= 0 && age <= 30_000;
    }
  }
  for (const perp of result.perps) {
    if (perp.state !== null) {
      const age = result.observedAt - perp.state.time;
      if (age > 30_000 || age < -5_000) {
        perp.sections.state.fresh = false;
        result.errors.push(`Perpetual state (${perp.dex || "main"}) has a stale or future exchange timestamp.`);
      }
    }
  }
  result.coverage.complete = sections.every((section) => section.fresh);
  return result;
}
