import { useEffect, useState, type ReactNode } from "react";
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";
import { addDecimals, compareDecimals, formatMoneyText } from "../hyperliquid/decimal.js";
import { InstrumentLabel } from "./instrument-label.js";
import type { DashboardSubagent } from "../agents/subagent-dashboard.js";
import { DashboardSubagents } from "./dashboard-subagents.js";
import { BalanceSparkline } from "./balance-sparkline.js";
import { DashboardChat } from "./dashboard-chat.js";

function Panel({ title, children }: { title: string; children: ReactNode }) {
  return <section className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6"><h3 className="mb-4 text-lg font-medium text-white">{title}</h3>{children}</section>;
}
function Statistic({ label, value, pnl = false }: { label: string; value: string | null; pnl?: boolean }) {
  const sign = value === null ? null : compareDecimals(value, "0");
  const color = pnl && sign !== null && sign !== 0 ? sign < 0 ? "text-red-400" : "text-emerald-400" : "text-zinc-100";
  return <div className="p-1"><dt className="text-xs uppercase tracking-[0.16em] text-zinc-500">{label}</dt><dd className="mt-1 flex items-baseline gap-2">{value === null ? <span className="text-base italic text-zinc-500">Unavailable</span> : <><span className={`font-mono text-2xl font-semibold tabular-nums ${color}`}>{formatMoneyText(value)}</span><span className="text-xs text-zinc-500">USDC</span></>}</dd></div>;
}
const cell = "whitespace-nowrap px-3 py-3 text-left";

export function AgentDashboardPage({ id, initial, subagents = [], subagentsError }: { id: string; initial: AgentDashboard; subagents?: DashboardSubagent[]; subagentsError?: string }) {
  const [snapshot, setSnapshot] = useState(initial);
  const [error, setError] = useState("");
  useEffect(() => {
    setSnapshot(initial);
    setError("");
    if (!initial.address) return;
    const source = new EventSource(`/api/agents/${id}/dashboard/events`);
    source.addEventListener("dashboard", (event) => {
      setSnapshot(JSON.parse((event as MessageEvent).data) as AgentDashboard);
      setError("");
    });
    source.addEventListener("unauthorized", () => { source.close(); window.location.assign("/login"); });
    const interrupted = () => setError("Live connection interrupted. Reconnecting… Displayed data is from the last snapshot, not current.");
    source.addEventListener("shutdown", interrupted);
    source.addEventListener("unavailable", interrupted);
    source.addEventListener("stream-error", () => { source.close(); setError("Account snapshot is too large to stream. Reload the page."); });
    source.onerror = interrupted;
    return () => source.close();
  }, [id, initial]);
  const connected = !error && snapshot.live?.status === "connected";
  const current = connected && Object.values(snapshot.live!.sections).every((section) => section.fresh) && snapshot.errors.length === 0;
  const connectionLabel = current ? "Connected" : connected ? "Connected · Data stale" : "Disconnected";
  const connectionColor = current ? "bg-emerald-400" : connected ? "bg-amber-400" : "bg-red-400";
  const totalEquity = snapshot.perpEquity === null || snapshot.spotUsdc === null ? null : addDecimals(snapshot.perpEquity, snapshot.spotUsdc);
  const unrealizedPnl = snapshot.positions?.reduce<string | null>((total, position) => total === null ? null : addDecimals(total, position.unrealizedPnl), "0") ?? null;
  return <div className="flex h-[calc(100dvh-4.5rem)] min-h-0 w-full flex-col gap-2 overflow-hidden sm:h-[calc(100dvh-5rem)]">
    <h2 className="sr-only">Dashboard</h2>
    <div className="max-h-[55%] shrink-0 space-y-2 overflow-y-auto">
    {!snapshot.address ? <Panel title="Trading account"><p className="text-sm text-zinc-400">This agent is missing its required Hyperliquid account. Create a new agent with a linked account.</p></Panel> : <>
      <section aria-label="Account overview" className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
        <p role="status" className="mb-5 flex items-center gap-2 text-xs text-zinc-400"><span aria-hidden="true" className={`h-2 w-2 rounded-full ${connectionColor}`} />{connectionLabel}</p>
        {error && <p role="alert" className="mb-4 rounded-xl border border-amber-900 p-3 text-sm text-amber-200">{error}</p>}
        {snapshot.errors.length > 0 && <div role="status" className="mb-4 rounded-xl border border-amber-900 p-3 text-sm text-amber-200">{snapshot.errors.map((message) => <p key={message}>{message}</p>)}</div>}
        <dl className="grid grid-cols-1 gap-6 sm:grid-cols-2 xl:grid-cols-4">
          <Statistic label="Total equity" value={totalEquity} />
          <Statistic label="Unrealized" value={unrealizedPnl} pnl />
          <BalanceSparkline label="24h" points={snapshot.balanceHistory?.day} />
          <BalanceSparkline label="30d" points={snapshot.balanceHistory?.month} />
        </dl>
      </section>
      <Panel title="Positions">{snapshot.positions === null ? <p className="text-sm text-amber-200">Positions unavailable.</p> : snapshot.positions.length === 0 ? <p className="text-sm text-zinc-500">No open positions.</p> : <div className="overflow-x-auto"><table className="w-full text-sm text-zinc-300"><thead className="text-xs text-zinc-500"><tr>{["Market", "Size (signed)", "Leverage", "Entry price", "Unrealized P&L", "Liquidation price"].map((label) => <th key={label} className={cell}>{label}</th>)}</tr></thead><tbody className="divide-y divide-zinc-800">{snapshot.positions.map((position) => <tr key={position.coin}><td className={cell}><InstrumentLabel symbol={position.coin} /></td><td className={cell}>{position.size}</td><td className={cell}>{position.leverage}×</td><td className={cell}>{position.entryPrice}</td><td className={cell}>{formatMoneyText(position.unrealizedPnl)}</td><td className={cell}>{position.liquidationPrice ?? "-"}</td></tr>)}</tbody></table></div>}</Panel>
      <Panel title="Open orders">{snapshot.orders === null ? <p className="text-sm text-amber-200">Orders unavailable.</p> : snapshot.orders.length === 0 ? <p className="text-sm text-zinc-500">No open orders.</p> : <div className="overflow-x-auto"><table className="w-full text-sm text-zinc-300"><thead className="text-xs text-zinc-500"><tr>{["Market", "Side", "Type", "Size", "Price / trigger limit"].map((label) => <th key={label} className={cell}>{label}</th>)}</tr></thead><tbody className="divide-y divide-zinc-800">{snapshot.orders.map((order) => <tr key={order.id}><td className={cell}><InstrumentLabel symbol={order.coin} /></td><td className={cell}>{order.side}</td><td className={cell}>{order.type}</td><td className={cell}>{order.size}</td><td className={cell}>{order.price}</td></tr>)}</tbody></table></div>}</Panel>
    </>}
    </div>
    <DashboardSubagents agentId={id} subagents={subagents} error={subagentsError} />
    <DashboardChat key={id} agentId={id} />
  </div>;
}
