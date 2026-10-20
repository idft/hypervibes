import type { BalancePoint } from "../hyperliquid/agent-dashboard.js";
import { amount } from "../hyperliquid/journal-decimal.js";
import { formatMoneyText } from "../hyperliquid/decimal.js";

export function BalanceSparkline({ label, points }: { label: string; points?: BalancePoint[] }) {
  const values = points?.map((point) => amount(point.balance)) ?? [];
  const first = values[0];
  const last = values.at(-1);
  const change = first && last ? last.sub(first) : null;
  let polyline = "";
  if (points && values.length > 1) {
    let min = values[0]!;
    let max = min;
    for (const value of values) {
      if (value.cmp(min) < 0) min = value;
      if (value.cmp(max) > 0) max = value;
    }
    const span = max.sub(min);
    const start = points[0]!.time;
    const duration = points.at(-1)!.time - start;
    polyline = values.map((value, index) => {
      // Only normalized drawing coordinates become floating point.
      const x = duration ? (points[index]!.time - start) / duration * 240 : 120;
      const y = span.zero ? 24 : 44 - Number(value.sub(min).div(span).toString()) * 40;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    }).join(" ");
  }
  const color = change && change.sign !== 0 ? change.sign < 0 ? "text-red-400" : "text-emerald-400" : "text-zinc-400";
  return <div className="min-w-0 p-1">
    <dt className="text-xs uppercase tracking-[0.16em] text-zinc-500">{label} balance</dt>
    <dd className="mt-2" title="Realized USDC balance, including transfers, fees and funding. Excludes unrealized P&L.">
      {polyline ? <>
        <span className={`font-mono text-sm tabular-nums ${color}`}>{change!.sign > 0 ? "+" : ""}{formatMoneyText(change!.toString())} USDC</span>
        <svg role="img" aria-label={`${label} realized account balance`} viewBox="0 0 240 48" className={`mt-2 h-12 w-full ${color}`} preserveAspectRatio="none"><title>{label} realized account balance (USDC)</title><polyline points={polyline} fill="none" stroke="currentColor" strokeWidth="2" vectorEffect="non-scaling-stroke" /></svg>
      </> : <span className="text-sm text-zinc-500">Balance history unavailable</span>}
    </dd>
  </div>;
}
