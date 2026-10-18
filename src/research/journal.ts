import type { SharedDatabase } from "../db/shared-database.js";
import type { JournalTrade } from "../hyperliquid/journal-model.js";
import { amount } from "../hyperliquid/journal-decimal.js";
import { AgentError } from "../agents/agent-input.js";

/** Reads only application journal tables; no private harness storage. */
export async function researchJournal(db: SharedDatabase, agentId: string, start: number, end: number, limit: number, before?: { time: number; id: string }) {
  if (![start, end, limit].every(Number.isSafeInteger) || start < 0 || end <= start || limit < 1 || limit > 1000 || (before && (!Number.isSafeInteger(before.time) || before.id.length > 128))) throw new AgentError("Invalid journal bounds");
  return db.transaction(async (tx) => {
    const agent = await tx.get<{ account: string | null }>("SELECT trading_account_address AS account FROM hv_agents WHERE id=?", agentId);
    if (!agent?.account) throw new AgentError("Agent has no trading account");
    const account = agent.account.toLowerCase();
    const rows = await tx.all<{ id: string; time: number; data: string }>(`SELECT id,time,json_remove(data,'$.parts') AS data FROM hv_journal_entries WHERE account=? AND kind='trade' AND active=1 AND time>=? AND time<? AND (time<? OR (time=? AND id<?)) ORDER BY time DESC,id DESC LIMIT ?`, account, start, end, before?.time ?? end, before?.time ?? end, before?.id ?? "", limit + 1);
    const sync = await tx.all<{ stream: string; cursor: number; updated_at: number | null; error: string | null }>("SELECT stream,cursor,updated_at,error FROM hv_journal_sync WHERE account=?", account);
    const state = await tx.get<{ revision: number; projected_revision: number; history_limited: number }>("SELECT revision,projected_revision,history_limited FROM hv_journal_accounts WHERE account=?", account);
    const complete = Boolean(state && !state.history_limited && state.revision === state.projected_revision && sync.length === 3 && sync.every((row) => row.updated_at && !row.error && row.cursor >= end));
    const trades = rows.slice(0, limit).map((row) => {
      const trade = JSON.parse(row.data) as Omit<JournalTrade, "parts">;
      return { ...trade, funding: complete ? trade.funding : null, net: complete ? trade.net : null };
    });
    const closed = trades.filter((trade) => trade.status === "closed");
    const knownNet = closed.filter((trade) => trade.net !== null);
    const sum = (values: string[]) => values.reduce((total, value) => total.add(amount(value)), amount("0")).toString();
    const tail = rows[Math.min(limit, rows.length) - 1];
    return { trades, next: rows.length > limit && tail ? { time: tail.time, id: tail.id } : null,
      performance: { scope: "returned closed trades only; not account equity/returns", closed: closed.length, knownNetCount: knownNet.length,
        knownNetSum: sum(knownNet.map((trade) => trade.net!)), knownNetWins: knownNet.filter((trade) => amount(trade.net!).sign > 0).length,
        grossPnl: sum(closed.map((trade) => trade.grossPnl)), fees: sum(closed.map((trade) => trade.fees)) },
      coverage: { complete, historyLimited: state?.history_limited === 1, sync, filter: "lastActivityAt in [start,end); persistent trade projections may change during sync", truncated: rows.length > limit } };
  });
}
