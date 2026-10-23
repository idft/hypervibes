import { createHash } from "node:crypto";
import type { UserFillsByTimeResponse, UserFundingResponse, UserNonFundingLedgerUpdatesResponse } from "@nktkas/hyperliquid";
import type { UserFundingsEvent } from "@nktkas/hyperliquid/api/subscription";
import { amount } from "./journal-decimal.js";

export type JournalStream = "fill" | "funding" | "ledger";
export interface JournalEvent {
  id: string; kind: JournalStream; time: number; symbol: string | null; type: string;
  fee: string | null; pnl: string | null; delta: string | null;
  fill: null | { hash: string; tid: string; side: string; size: string; price: string; start: string | null; feeToken: string | null; rawFee: string | null };
  raw: unknown;
}
export interface JournalTrade {
  id: string; symbol: string; direction: "long" | "short" | "unknown"; status: "open" | "closed" | "incomplete";
  openedAt: number; lastActivityAt: number; closedAt: number | null;
  remainingSize: string; entrySize: string; exitSize: string; entryNotional: string; exitNotional: string;
  grossPnl: string; fees: string; funding: string | null; net: string | null; feeCoverage: boolean; coverageReason: string | null;
  parts: { eventId: string; time: number; price: string; side: string; segment: "entry" | "exit" | "flip_entry"; quantity: string; fee: string | null; pnl: string }[];
}
export function stableId(...parts: string[]) { return createHash("sha256").update(JSON.stringify(parts)).digest("hex"); }
const decimal = (value: string | undefined): string | null => value === undefined ? null : amount(value).toString();

export function fillEvent(row: UserFillsByTimeResponse[number]): JournalEvent {
  if (!Number.isSafeInteger(row.tid)) throw new Error("Exchange trade identity exceeds safe integer range");
  const fee = row.feeToken?.toUpperCase() === "USDC" ? decimal(row.fee) : null;
  const pnl = decimal(row.closedPnl);
  // Spot principal and non-USDC fees must never masquerade as a USDC cash delta.
  const perp = !row.coin.startsWith("@") && !row.coin.includes("/");
  return { id: stableId("fill", row.hash, String(row.tid)), kind: "fill", time: row.time, symbol: row.coin,
    type: `fill ${row.side}`, fee, pnl, delta: perp && pnl !== null && fee !== null ? amount(pnl).sub(amount(fee)).toString() : null,
    fill: perp ? { hash: row.hash, tid: String(row.tid), side: row.side, size: amount(row.sz).toString(), price: amount(row.px).toString(), start: decimal(row.startPosition), feeToken: row.feeToken ?? null, rawFee: decimal(row.fee) } : null, raw: row };
}
export function fundingEvent(row: UserFundingResponse[number]): JournalEvent {
  return { id: stableId("funding", row.delta.coin, String(row.time)), kind: "funding", time: row.time, symbol: row.delta.coin,
    type: "funding", fee: null, pnl: null, delta: amount(row.delta.usdc).toString(), fill: null, raw: row };
}
/** WebSocket funding pushes carry a flat shape but must share HTTP event identities. */
export function wsFundingEvent(row: UserFundingsEvent["fundings"][number]): JournalEvent {
  return { id: stableId("funding", row.coin, String(row.time)), kind: "funding", time: row.time, symbol: row.coin,
    type: "funding", fee: null, pnl: null, delta: amount(row.usdc).toString(), fill: null, raw: row };
}
export function ledgerEvent(account: string, row: UserNonFundingLedgerUpdatesResponse[number]): JournalEvent {
  const d = row.delta;
  let delta: string | null = null;
  let fee: string | null = null;
  if (d.type === "deposit") delta = amount(d.usdc).toString();
  if (d.type === "withdraw") delta = amount(d.usdc).neg().toString();
  if (d.type === "send" && d.token.toUpperCase() === "USDC" && amount(d.amount).sign >= 0) {
    const sender = d.user.toLowerCase() === account;
    const recipient = d.destination.toLowerCase() === account;
    if ((sender || recipient) && (amount(d.fee).zero || d.feeToken.toUpperCase() === "USDC")) {
      fee = sender ? amount(d.fee).toString() : "0";
      delta = (sender === recipient ? amount("0") : sender ? amount(d.amount).neg() : amount(d.amount)).sub(amount(fee)).toString();
    }
  }
  return { id: stableId("ledger", row.hash, String(row.time), d.type), kind: "ledger", time: row.time,
    symbol: "token" in d ? d.token : "usdc" in d ? "USDC" : null, type: d.type, fee, pnl: null, delta, fill: null, raw: row };
}

/** Exchange P&L, position-chain ordering, and flat-to-flat perps. */
export function buildTrades(account: string, events: JournalEvent[]): JournalTrade[] {
  const result: JournalTrade[] = [];
  const instruments = new Map<string, JournalEvent[]>();
  for (const event of events) if (event.fill && event.symbol) {
    const fills = instruments.get(event.symbol) ?? []; fills.push(event); instruments.set(event.symbol, fills);
  }
  for (const [symbol, fills] of instruments) {
    fills.sort((a, b) => a.time - b.time || compareTid(a.fill!.tid, b.fill!.tid) || a.fill!.hash.localeCompare(b.fill!.hash));
    let signed = amount("0");
    let active: JournalTrade | undefined;
    const incomplete = (reason: string) => { if (active) { active.status = "incomplete"; active.coverageReason = reason; } };
    const create = (event: JournalEvent, segment: string, direction: JournalTrade["direction"], partial: boolean): JournalTrade => ({
      id: stableId(account, "live", symbol, direction, event.fill!.hash, event.fill!.tid, segment), symbol, direction,
      status: partial ? "incomplete" : "open", openedAt: event.time, lastActivityAt: event.time, closedAt: null,
      remainingSize: "0", entrySize: "0", exitSize: "0", entryNotional: "0", exitNotional: "0", grossPnl: "0", fees: "0", funding: null, net: null,
      feeCoverage: true, coverageReason: partial ? "Position predates or disagrees with available fills" : null, parts: [],
    });
    const part = (trade: JournalTrade, event: JournalEvent, segment: "entry" | "exit" | "flip_entry", quantity: string, fee: string | null, pnl: string) => {
      const sizeKey = segment === "exit" ? "exitSize" : "entrySize";
      const notionalKey = segment === "exit" ? "exitNotional" : "entryNotional";
      trade[sizeKey] = amount(trade[sizeKey]).add(amount(quantity)).toString();
      trade[notionalKey] = amount(trade[notionalKey]).add(amount(quantity).mul(amount(event.fill!.price))).toString();
      trade.grossPnl = amount(trade.grossPnl).add(amount(pnl)).toString();
      if (fee !== null) trade.fees = amount(trade.fees).add(amount(fee)).toString();
      else { trade.feeCoverage = false; trade.coverageReason ??= "Fee is missing or not denominated in USDC"; }
      trade.lastActivityAt = event.time;
      trade.parts.push({ eventId: event.id, time: event.time, price: event.fill!.price, side: event.fill!.side, segment, quantity, fee, pnl });
    };
    let groupEnd = 0;
    for (let index = 0; index < fills.length; index++) {
      if (index === groupEnd) { groupEnd = index + 1; while (groupEnd < fills.length && fills[groupEnd]!.time === fills[index]!.time) groupEnd++; }
      const next = fills.slice(index, groupEnd).findIndex((f) => f.fill!.start !== null && amount(f.fill!.start!).cmp(signed) === 0);
      if (next > 0) fills.splice(index, 0, ...fills.splice(index + next, 1));
      const event = fills[index]!; const fill = event.fill!;
      const size = amount(fill.size);
      if (size.sign <= 0 || amount(fill.price).sign < 0) throw new Error("Invalid journal fill size or price");
      const sign = fill.side === "B" || fill.side === "buy" ? 1 : fill.side === "A" || fill.side === "sell" ? -1 : 0;
      if (!sign) {
        if (active) { incomplete("Unrecognized exchange side"); result.push(active); active = undefined; }
        const unknown = create(event, "entry", "unknown", true); unknown.coverageReason = "Unrecognized exchange side"; unknown.remainingSize = fill.size;
        part(unknown, event, "entry", fill.size, null, "0"); result.push(unknown); signed = amount("0"); continue;
      }
      if (fill.start !== null && amount(fill.start).cmp(signed) !== 0) {
        if (active) { incomplete("Exchange startPosition disagrees with reconstructed size"); result.push(active); active = undefined; }
        signed = amount(fill.start);
        if (!signed.zero) { active = create(event, "entry", signed.sign > 0 ? "long" : "short", true); active.remainingSize = signed.abs().toString(); }
      } else if (fill.start === null) incomplete("Exchange startPosition missing");
      const closing = !signed.zero && signed.sign !== sign ? (signed.abs().cmp(size) < 0 ? signed.abs() : size) : amount("0");
      const opening = size.sub(closing);
      const closingFee = event.fee === null ? null : amount(event.fee).mul(closing).div(size).toString();
      if (!closing.zero) {
        if (!active) throw new Error("Missing active journal trade");
        if (event.pnl === null) incomplete("Exchange closing P&L missing");
        part(active, event, "exit", closing.toString(), closingFee, event.pnl ?? "0");
        signed = signed.add(sign > 0 ? closing : closing.neg()); active.remainingSize = signed.abs().toString();
        if (signed.zero) { active.closedAt = event.time; if (active.status === "open") active.status = "closed"; result.push(active); active = undefined; }
      }
      if (!opening.zero) {
        const segment = closing.zero ? "entry" : "flip_entry";
        active ??= create(event, segment, sign > 0 ? "long" : "short", fill.start === null);
        signed = signed.add(sign > 0 ? opening : opening.neg()); active.remainingSize = signed.abs().toString();
        part(active, event, segment, opening.toString(), event.fee === null ? null : amount(event.fee).sub(amount(closingFee!)).toString(), "0");
      }
    }
    if (active) result.push(active);
  }
  for (const trade of result) {
    if (trade.status !== "incomplete") trade.funding = events.filter((e) => e.kind === "funding" && e.symbol === trade.symbol && e.time >= trade.openedAt && (trade.closedAt === null || e.time < trade.closedAt))
      .reduce((sum, e) => sum.add(amount(e.delta!)), amount("0")).toString();
    if (trade.feeCoverage && trade.funding !== null) trade.net = amount(trade.grossPnl).sub(amount(trade.fees)).add(amount(trade.funding)).toString();
  }
  return result;
}
function compareTid(a: string, b: string) { if (/^\d+$/.test(a) && /^\d+$/.test(b)) return BigInt(a) < BigInt(b) ? -1 : BigInt(a) > BigInt(b) ? 1 : 0; return a.localeCompare(b); }
