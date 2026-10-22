import { useEffect, useState, type ReactNode } from "react";
import { useAgentEvent, useAgentEventsStatus } from "./agent-events.js";
import type { AgentDashboard } from "../hyperliquid/agent-dashboard.js";
import { addDecimals, compareDecimals, formatAmountText, formatMoneyText } from "../hyperliquid/decimal.js";
import { InstrumentLabel } from "./instrument-label.js";
import type { DashboardSubagent } from "../agents/subagent-dashboard.js";
import { DashboardSubagents } from "./dashboard-subagents.js";
import { BalanceSparkline } from "./balance-sparkline.js";
import { ClosePositionButton } from "./close-position-button.js";
import { DashboardChat } from "./dashboard-chat.js";

function Panel({ title, children }: { title: string; children: ReactNode }) {
  return <section className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6"><h3 className="mb-4 text-lg font-medium text-white">{title}</h3>{children}</section>;
}
function pnlColor(value: string | null): string | null {
  const sign = value === null ? null : compareDecimals(value, "0");
  return sign === null || sign === 0 ? null : sign < 0 ? "text-red-400" : "text-emerald-400";
}
function positionSizeText(value: string): string {
  const sign = compareDecimals(value, "0");
  if (sign === null || sign === 0) return formatAmountText(value);
  return `${sign < 0 ? "Short" : "Long"} ${formatAmountText(value.replace(/^-/, ""))}`;
}
function Statistic({ label, value, pnl = false }: { label: string; value: string | null; pnl?: boolean }) {
  const color = (pnl ? pnlColor(value) : null) ?? "text-zinc-100";
  return <div className="p-1"><dt className="text-xs uppercase tracking-[0.16em] text-zinc-500">{label}</dt><dd className="mt-1 flex items-baseline gap-2">{value === null ? <span className="text-base italic text-zinc-500">Unavailable</span> : <><span className={`font-mono text-2xl font-semibold tabular-nums ${color}`}>{formatMoneyText(value)}</span><span className="text-xs text-zinc-500">USDC</span></>}</dd></div>;
}
const cell = "whitespace-nowrap px-3 py-3 text-left";

const noSubagents: DashboardSubagent[] = [];

export function AgentDashboardPage({ id, initial, enabled = true, subagents = noSubagents, subagentsError }: { id: string; initial: AgentDashboard; enabled?: boolean; subagents?: DashboardSubagent[]; subagentsError?: string }) {
  const [snapshot, setSnapshot] = useState(initial);
  const [error, setError] = useState("");
  const { interrupted, controlMessage, streamErrors } = useAgentEventsStatus();
  const hasAddress = Boolean(initial.address);
  useAgentEvent("dashboard", (event) => {
    setSnapshot(JSON.parse(event.data) as AgentDashboard);
    setError("");
  }, hasAddress);
  useEffect(() => {
    setSnapshot(initial);
    setError("");
  }, [id, initial]);
  useEffect(() => {
    if (!hasAddress) return;
    const topicError = streamErrors["dashboard"];
    if (topicError) setError(topicError);
    else if (interrupted) setError(controlMessage || "Live connection interrupted. Reconnecting… Displayed data is from the last snapshot, not current.");
  }, [hasAddress, interrupted, controlMessage, streamErrors]);
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
      <Panel title="Positions">{snapshot.positions === null ? <p className="text-sm text-amber-200">Positions unavailable.</p> : snapshot.positions.length === 0 ? <p className="text-sm text-zinc-500">No open positions.</p> : <div className="overflow-x-auto"><table className="w-full text-sm text-zinc-300"><thead className="text-xs text-zinc-500"><tr>{["Market", "Size", "Leverage", "Entry price", "Unrealized P&L", "Liquidation price", "Actions"].map((label) => <th key={label} className={cell}>{label}</th>)}</tr></thead><tbody className="divide-y divide-zinc-800">{snapshot.positions.map((position) => <tr key={position.coin}><td className={cell}><InstrumentLabel symbol={position.coin} /></td><td className={cell}><span className="font-mono tabular-nums">{positionSizeText(position.size)}</span></td><td className={cell}>{position.leverage}×</td><td className={cell}><span className="font-mono tabular-nums">{formatAmountText(position.entryPrice)}</span></td><td className={cell}><span className={`font-mono tabular-nums ${pnlColor(position.unrealizedPnl) ?? "text-zinc-300"}`}>{formatMoneyText(position.unrealizedPnl)}</span></td><td className={cell}><span className="font-mono tabular-nums">{formatMoneyText(position.liquidationPrice, 2)}</span></td><td className={cell}><ClosePositionButton agentId={id} symbol={position.coin} /></td></tr>)}</tbody></table></div>}</Panel>
      <Panel title="Open orders">{snapshot.orders === null ? <p className="text-sm text-amber-200">Orders unavailable.</p> : snapshot.orders.length === 0 ? <p className="text-sm text-zinc-500">No open orders.</p> : <div className="overflow-x-auto"><table className="w-full text-sm text-zinc-300"><thead className="text-xs text-zinc-500"><tr>{["Market", "Side", "Type", "Size", "Price / trigger"].map((label) => <th key={label} className={cell}>{label}</th>)}</tr></thead><tbody className="divide-y divide-zinc-800">{snapshot.orders.map((order) => <tr key={order.id}><td className={cell}><InstrumentLabel symbol={order.coin} /></td><td className={cell}>{order.side}</td><td className={cell}>{order.type}</td><td className={cell}><span className="font-mono tabular-nums">{formatAmountText(order.size)}</span></td><td className={cell}><span className="font-mono tabular-nums">{formatAmountText(order.triggerPrice ?? order.price)}</span></td></tr>)}</tbody></table></div>}</Panel>
    </>}
    </div>
    <DashboardSubagents agentId={id} agentEnabled={enabled} subagents={subagents} error={subagentsError} />
    <DashboardChat key={id} agentId={id} enabled={enabled} />
  </div>;
}
