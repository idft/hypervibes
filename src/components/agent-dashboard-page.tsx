import { useEffect, useState, type ReactNode } from "react";
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";
import { addDecimals, compareDecimals, formatMoneyText } from "../hyperliquid/decimal.js";
import { getAgentDashboard } from "../web/agents.js";
import { AgentSectionPage } from "./agent-section-page.js";

function Panel({ title, children }: { title: string; children: ReactNode }) {
  return <section className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6"><h3 className="mb-4 text-lg font-medium text-white">{title}</h3>{children}</section>;
}
function Statistic({ label, value, pnl = false }: { label: string; value: string | null; pnl?: boolean }) {
  const sign = value === null ? null : compareDecimals(value, "0");
  const color = pnl && sign !== null && sign !== 0 ? sign < 0 ? "text-red-400" : "text-emerald-400" : "text-zinc-100";
  return <div className="p-1"><dt className="text-xs uppercase tracking-[0.16em] text-zinc-500">{label}</dt><dd className="mt-1 flex items-baseline gap-2">{value === null ? <span className="text-base italic text-zinc-500">Unavailable</span> : <><span className={`font-mono text-2xl font-semibold tabular-nums ${color}`}>{formatMoneyText(value)}</span><span className="text-xs text-zinc-500">USDC</span></>}</dd></div>;
}
const cell = "whitespace-nowrap px-3 py-3 text-left";

export function AgentDashboardPage({ id, initial }: { id: string; initial: AgentDashboard }) {
  const [snapshot, setSnapshot] = useState(initial);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!initial.address) return;
    let stopped = false;
    let pending = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => { if (!stopped) timer = setTimeout(() => { void refresh(); }, 15_000); };
    async function refresh() {
      if (stopped || pending) return;
      clearTimeout(timer);
      if (document.hidden) { schedule(); return; }
      pending = true;
      try {
        const result = await getAgentDashboard({ data: { id } });
        if (stopped) return;
        if (result.ok) { setSnapshot(result.data); setError(""); }
        else setError(result.error);
      } catch { if (!stopped) setError("Refresh failed. Displayed data is from the last snapshot, not current."); }
      finally { pending = false; schedule(); }
    }
    const visible = () => { if (!document.hidden) void refresh(); };
    document.addEventListener("visibilitychange", visible);
    schedule();
    return () => { stopped = true; clearTimeout(timer); document.removeEventListener("visibilitychange", visible); };
  }, [id, initial.address]);
  const unrealizedPnl = snapshot.positions?.reduce<string | null>((total, position) => total === null ? null : addDecimals(total, position.unrealizedPnl), "0") ?? null;
  return <div className="w-full space-y-2">
    <h2 className="sr-only">Dashboard</h2>
    {!snapshot.address ? <Panel title="Trading account"><p className="text-sm text-zinc-400">No trading account is linked to this Agent. Chat is available without one.</p></Panel> : <>
      <section aria-label="Account overview" className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
        <div className="mb-5 flex flex-wrap items-center justify-between gap-2 text-xs text-zinc-500">
          <p className="min-w-0 break-all font-mono">{snapshot.address}</p>
          <p>Last fetched: <time dateTime={new Date(snapshot.fetchedAt).toISOString()}>{new Date(snapshot.fetchedAt).toISOString()}</time></p>
        </div>
        {error && <p role="alert" className="mb-4 rounded-xl border border-amber-900 p-3 text-sm text-amber-200">{error}</p>}
        {snapshot.errors.length > 0 && <div role="status" className="mb-4 rounded-xl border border-amber-900 p-3 text-sm text-amber-200">{snapshot.errors.map((message) => <p key={message}>{message}</p>)}</div>}
        <dl className="grid grid-cols-1 gap-6 sm:grid-cols-2 xl:grid-cols-4">
          <Statistic label="Perpetual equity" value={snapshot.perpEquity} />
          <Statistic label="Unrealized" value={unrealizedPnl} pnl />
          <Statistic label="Spot USDC" value={snapshot.spotUsdc} />
          <div className="p-1"><dt className="text-xs uppercase tracking-[0.16em] text-zinc-500">24h performance</dt><dd className="mt-1 text-sm text-zinc-600">History not available yet</dd></div>
        </dl>
        <p className="mt-5 border-t border-zinc-800 pt-3 text-xs text-zinc-500">Read-only · refreshes every 15 seconds while visible · main perpetual DEX only. Equity and spot balances are separate, not an aggregated portfolio valuation.</p>
      </section>
      <Panel title="Positions">{snapshot.positions === null ? <p className="text-sm text-amber-200">Positions unavailable.</p> : snapshot.positions.length === 0 ? <p className="text-sm text-zinc-500">No open positions.</p> : <div className="overflow-x-auto"><table className="w-full text-sm text-zinc-300"><thead className="text-xs text-zinc-500"><tr>{["Market", "Size (signed)", "Leverage", "Entry price", "Unrealized P&L", "Liquidation price"].map((label) => <th key={label} className={cell}>{label}</th>)}</tr></thead><tbody className="divide-y divide-zinc-800">{snapshot.positions.map((position) => <tr key={position.coin}><td className={cell}>{position.coin}</td><td className={cell}>{position.size}</td><td className={cell}>{position.leverage}×</td><td className={cell}>{position.entryPrice}</td><td className={cell}>{formatMoneyText(position.unrealizedPnl)}</td><td className={cell}>{position.liquidationPrice ?? "-"}</td></tr>)}</tbody></table></div>}</Panel>
      <Panel title="Open orders">{snapshot.orders === null ? <p className="text-sm text-amber-200">Orders unavailable.</p> : snapshot.orders.length === 0 ? <p className="text-sm text-zinc-500">No open orders.</p> : <div className="overflow-x-auto"><table className="w-full text-sm text-zinc-300"><thead className="text-xs text-zinc-500"><tr>{["Market", "Side", "Type", "Size", "Price / trigger limit"].map((label) => <th key={label} className={cell}>{label}</th>)}</tr></thead><tbody className="divide-y divide-zinc-800">{snapshot.orders.map((order) => <tr key={order.id}><td className={cell}>{order.coin}</td><td className={cell}>{order.side}</td><td className={cell}>{order.type}</td><td className={cell}>{order.size}</td><td className={cell}>{order.price}</td></tr>)}</tbody></table></div>}</Panel>
    </>}
    <AgentSectionPage title="Performance" description="Historical balance and P&L charts are not available yet." />
    <AgentSectionPage title="Trade journal" description="Trade history, activity, and journal notes are not available yet." />
  </div>;
}
