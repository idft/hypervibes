import { useState } from "react";
import { toast } from "sonner";
import type { AgentChat } from "../agents/agent-service.js";
import type { RiskSettings } from "../hyperliquid/orders/risk-settings.js";
import { saveRiskSettings } from "../web/risk.js";
import { AgentSectionPage } from "./agent-section-page.js";

const input = "mt-2 block w-full rounded-lg border border-zinc-700 bg-zinc-950 p-3 text-sm text-white";
const button = "cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50";

/** The only host-enforced trading rules; everything else is instructions/learnings. */
function RiskManagementSection({ agentId, initial }: { agentId: string; initial?: RiskSettings }) {
  const [enabled, setEnabled] = useState(initial?.tradingEnabled ?? false);
  const [instruments, setInstruments] = useState(initial?.allowedInstruments?.join(", ") ?? "");
  const [maxOrder, setMaxOrder] = useState(initial?.maxOrderNotional ?? "");
  const [maxTotal, setMaxTotal] = useState(initial?.maxTotalNotional ?? "");
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  async function save() {
    setBusy(true); setError("");
    try {
      const result = await saveRiskSettings({ data: { agentId, settings: {
        tradingEnabled: enabled,
        allowedInstruments: instruments.trim() ? instruments.split(",").map((coin) => coin.trim()).filter(Boolean) : null,
        maxOrderNotional: maxOrder.trim() || null,
        maxTotalNotional: maxTotal.trim() || null,
      } } });
      if (!result.ok) throw new Error(result.error);
      toast.success("Risk settings saved");
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save risk settings; your draft is preserved"); }
    finally { setBusy(false); }
  }
  return <section className="w-full rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6" aria-label="Risk management">
    <h2 className="text-lg font-medium text-white">Risk management</h2>
    <p className="mt-2 text-sm text-zinc-400">The only application-enforced trading rules. Opening orders are rejected unless trading is enabled, the instrument is allowed, and the notional caps hold; reduce-only management and cancellation remain available. Strategy, sizing guidance and TP/SL levels live in worker instructions and learnings.</p>
    <fieldset disabled={busy} className="mt-4 space-y-4">
      <label className="flex cursor-pointer items-center gap-2 text-sm"><input type="checkbox" className="cursor-pointer" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />Enable trading (new exposure is rejected while off)</label>
      <label className="block text-sm text-zinc-300">Allowed instruments (exact main-DEX symbols, comma separated; empty = no limit)<input aria-label="Allowed instruments" className={input} maxLength={330} placeholder="BTC, ETH" value={instruments} onChange={(event) => setInstruments(event.target.value)} /></label>
      <div className="grid gap-4 sm:grid-cols-2">
        <label className="block text-sm text-zinc-300">Max order notional (USDC; empty = uncapped)<input aria-label="Max order notional" className={input} inputMode="decimal" placeholder="1000" value={maxOrder} onChange={(event) => setMaxOrder(event.target.value)} /></label>
        <label className="block text-sm text-zinc-300">Max total exposure notional (USDC; empty = uncapped)<input aria-label="Max total notional" className={input} inputMode="decimal" placeholder="5000" value={maxTotal} onChange={(event) => setMaxTotal(event.target.value)} /></label>
      </div>
      <p className="text-xs text-zinc-400">Total exposure includes positions at current mids plus pending entry orders at the higher of mid or limit price. Recent local fills reserve capacity for 30 seconds while account snapshots catch up and may briefly be counted twice. These are admission limits, not a guarantee against later price moves. Saving limits does not cancel existing orders or close positions.</p>
      <button type="button" className={button} onClick={() => { void save(); }}>{busy ? "Saving…" : "Save risk settings"}</button>
    </fieldset>
    {error && <p role="alert" className="mt-3 text-sm text-red-300">{error}</p>}
  </section>;
}

export function AgentSettingsPage({ initial, risk }: { initial: AgentChat; risk?: RiskSettings }) {
  return <div className="w-full space-y-2">
    <section className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
      <h2 className="text-lg font-medium text-white">Settings</h2>
      <dl className="mt-4 grid gap-4 sm:grid-cols-2"><div><dt className="text-xs uppercase tracking-wide text-zinc-500">Trading account</dt><dd className="mt-2 break-all font-mono text-sm text-zinc-300">{initial.agent.tradingAccountAddress ?? "No account linked (chat only)"}</dd></div><div><dt className="text-xs uppercase tracking-wide text-zinc-500">Agent status</dt><dd className="mt-2 text-sm text-zinc-300">{initial.agent.state}</dd></div></dl>
      <a href={`/agents/${initial.agent.id}/sub-agents/${encodeURIComponent(`main:${initial.agent.id}`)}`} className="mt-5 inline-block cursor-pointer text-sm text-violet-300 hover:underline">Edit orchestrator model, thinking, instructions & tools →</a>
    </section>
    <RiskManagementSection agentId={initial.agent.id} initial={risk} />
    <AgentSectionPage title="Execution backend" description="Execution environment configuration will be available here." />
    <AgentSectionPage title="Telegram" description="Connect Telegram to this Agent’s main chat in a future update." />
  </div>;
}