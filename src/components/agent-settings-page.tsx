import { useRouter } from "@tanstack/react-router";
import { useId, useState } from "react";
import { toast } from "sonner";
import type { AgentChat } from "../agents/agent-service.js";
import type { RiskSettings } from "../hyperliquid/orders/risk-settings.js";
import { deleteAgent } from "../web/agents.js";
import { saveRiskSettings } from "../web/risk.js";
import { AgentSectionPage } from "./agent-section-page.js";
import { InstrumentPicker } from "./instrument-picker.js";

const button = "cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50";

interface NotionalLimitDraft { limited: boolean; amount: string }
const radio = "size-4 shrink-0 cursor-pointer appearance-none rounded-full border border-zinc-600 bg-zinc-950 transition checked:border-[5px] checked:border-violet-500 checked:bg-white hover:border-violet-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-950 disabled:cursor-wait disabled:opacity-50";

function NotionalLimit({ title, inputLabel, placeholder, value, onChange }: { title: string; inputLabel: string; placeholder: string; value: NotionalLimitDraft; onChange: (value: NotionalLimitDraft) => void }) {
  const id = useId();
  return <fieldset className="min-w-0 space-y-3 text-sm text-zinc-300">
    <legend className="mb-3 font-medium">{title}</legend>
    <label className="flex w-fit cursor-pointer items-center gap-2">
      <input type="radio" name={id} value="unlimited" className={radio} checked={!value.limited} onChange={() => onChange({ ...value, limited: false })} />No limit
    </label>
    <div className="flex items-center gap-3">
      <label className="flex shrink-0 cursor-pointer items-center gap-2">
        <input type="radio" name={id} value="limited" className={radio} checked={value.limited} onChange={() => onChange({ ...value, limited: true })} />Limit:
      </label>
      <div className={`relative min-w-0 flex-1 ${value.limited ? "" : "opacity-50"}`}>
        <input aria-label={inputLabel} aria-describedby={`${id}-currency`} className="block w-full rounded-lg border border-zinc-700 bg-zinc-950 py-2.5 pl-3 pr-16 text-sm text-white focus-visible:border-violet-400 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-violet-400 disabled:cursor-not-allowed" inputMode="decimal" placeholder={placeholder} disabled={!value.limited} value={value.amount} onChange={(event) => onChange({ ...value, amount: event.target.value })} />
        <span id={`${id}-currency`} className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-xs text-zinc-500">USDC</span>
      </div>
    </div>
  </fieldset>;
}

/** The only host-enforced trading rules; everything else is instructions/insights. */
function RiskManagementSection({ agentId, initial }: { agentId: string; initial?: RiskSettings }) {
  const router = useRouter();
  const [enabled, setEnabled] = useState(initial?.tradingEnabled ?? false);
  const [instruments, setInstruments] = useState<string[] | null>(initial?.allowedInstruments ?? null);
  const [maxOrder, setMaxOrder] = useState<NotionalLimitDraft>({ limited: initial?.maxOrderNotional != null, amount: initial?.maxOrderNotional ?? "" });
  const [maxTotal, setMaxTotal] = useState<NotionalLimitDraft>({ limited: initial?.maxTotalNotional != null, amount: initial?.maxTotalNotional ?? "" });
  const [saved, setSaved] = useState<RiskSettings>(initial ?? { tradingEnabled: false, allowedInstruments: null, maxOrderNotional: null, maxTotalNotional: null });
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  const settings: RiskSettings = {
    tradingEnabled: enabled,
    allowedInstruments: instruments,
    maxOrderNotional: maxOrder.limited ? maxOrder.amount.trim() : null,
    maxTotalNotional: maxTotal.limited ? maxTotal.amount.trim() : null,
  };
  const dirty = settings.tradingEnabled !== saved.tradingEnabled
    || JSON.stringify(settings.allowedInstruments === null ? null : [...settings.allowedInstruments].sort()) !== JSON.stringify(saved.allowedInstruments === null ? null : [...saved.allowedInstruments].sort())
    || settings.maxOrderNotional !== saved.maxOrderNotional
    || settings.maxTotalNotional !== saved.maxTotalNotional;
  async function save() {
    if (busy || !dirty) return;
    setBusy(true); setError("");
    try {
      if (maxOrder.limited && !maxOrder.amount.trim()) throw new Error("Enter a max order notional limit or select No limit.");
      if (maxTotal.limited && !maxTotal.amount.trim()) throw new Error("Enter a max total exposure limit or select No limit.");
      const result = await saveRiskSettings({ data: { agentId, settings } });
      if (!result.ok) throw new Error(result.error);
      setSaved(result.data);
      toast.success("Risk settings saved");
      await router.invalidate();
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save risk settings; your draft is preserved"); }
    finally { setBusy(false); }
  }
  return <section id="risk-management" className="w-full scroll-mt-2 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6" aria-label="Risk management">
    <h2 className="text-lg font-medium text-white">Risk management</h2>
    <fieldset disabled={busy} className="mt-4 space-y-4">
      <label className="flex cursor-pointer items-center gap-3 text-sm text-zinc-300">
        <span className="relative inline-flex size-5 shrink-0">
          <input type="checkbox" className="peer size-5 cursor-pointer appearance-none rounded border border-zinc-600 bg-zinc-950 transition checked:border-violet-500 checked:bg-violet-600 hover:border-violet-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-400 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-950 disabled:cursor-wait disabled:opacity-50" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />
          <svg aria-hidden="true" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth={2.5} className="pointer-events-none absolute inset-0.5 size-4 text-white opacity-0 transition peer-checked:opacity-100 peer-disabled:text-zinc-400"><path strokeLinecap="round" strokeLinejoin="round" d="m4 10 4 4 8-8" /></svg>
        </span>
        <span>Enable Trading</span>
      </label>
      <InstrumentPicker agentId={agentId} value={instruments} onChange={setInstruments} disabled={busy} />
      <div className="grid gap-4 sm:grid-cols-2">
        <NotionalLimit title="Max order notional" inputLabel="Max order notional" placeholder="1000" value={maxOrder} onChange={setMaxOrder} />
        <NotionalLimit title="Max total exposure" inputLabel="Max total notional" placeholder="5000" value={maxTotal} onChange={setMaxTotal} />
      </div>
      {dirty && <button type="button" className={button} disabled={busy} onClick={() => { void save(); }}>{busy ? "Saving…" : "Save risk settings"}</button>}
    </fieldset>
    {error && <p role="alert" className="mt-3 text-sm text-red-300">{error}</p>}
  </section>;
}

function DeleteAgentSection({ agent }: { agent: AgentChat["agent"] }) {
  const router = useRouter();
  const [confirming, setConfirming] = useState(false);
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function remove() {
    if (busy || confirmation !== agent.name) return;
    setBusy(true); setError("");
    try {
      const result = await deleteAgent({ data: { id: agent.id, confirmation } });
      if (!result.ok) throw new Error(result.error);
      await router.navigate({ to: "/agents" });
      await router.invalidate();
      toast.success("Agent deleted");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Deletion incomplete. Retry deletion.");
    } finally { setBusy(false); }
  }
  return <section aria-label="Danger zone" className="rounded-2xl border border-red-900/60 bg-[#0d0d10] p-6">
    <h2 className="text-lg font-medium text-red-300">Danger zone</h2>
    <p className="mt-3 text-sm text-zinc-400">Deleting this agent stops its workers, removes its configuration and application history, and releases its Hyperliquid account for reuse. Existing exchange orders and positions are not changed. This cannot be undone.</p>
    {agent.error && (agent.state === "stopping" || agent.state === "stopped") && <p role="status" className="mt-3 text-sm text-red-300">{agent.error}</p>}
    {!confirming ? <button type="button" onClick={() => setConfirming(true)} className="mt-4 cursor-pointer rounded-full bg-red-700 px-4 py-2 text-sm text-white hover:bg-red-600">Delete agent</button> : <form className="mt-4 space-y-3" onSubmit={(event) => { event.preventDefault(); void remove(); }}>
      <label className="block text-sm text-zinc-300">Type <strong>{agent.name}</strong> to confirm deletion
        <input aria-label="Agent name confirmation" autoComplete="off" value={confirmation} disabled={busy} onChange={(event) => setConfirmation(event.target.value)} className="mt-2 block w-full rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-white" />
      </label>
      <div className="flex gap-3">
        <button type="button" disabled={busy} onClick={() => { setConfirming(false); setConfirmation(""); setError(""); }} className="cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 disabled:cursor-not-allowed">Cancel</button>
        <button type="submit" disabled={busy || confirmation !== agent.name} className="cursor-pointer rounded-full bg-red-700 px-4 py-2 text-sm text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:opacity-50">{busy ? "Stopping and deleting…" : "Permanently delete agent"}</button>
      </div>
    </form>}
    {error && <p role="alert" className="mt-3 text-sm text-red-300">{error}</p>}
  </section>;
}

export function AgentSettingsPage({ initial, risk }: { initial: AgentChat; risk?: RiskSettings }) {
  return <div className="w-full space-y-2">
    <section className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
      <h2 className="text-lg font-medium text-white">Settings</h2>
      <dl className="mt-4 grid gap-4 sm:grid-cols-2"><div><dt className="text-xs uppercase tracking-wide text-zinc-500">Trading account</dt><dd className="mt-2 break-all font-mono text-sm text-zinc-300">{initial.agent.tradingAccountAddress ?? "No account linked (chat only)"}</dd></div><div><dt className="text-xs uppercase tracking-wide text-zinc-500">Agent status</dt><dd className="mt-2 text-sm text-zinc-300">{initial.agent.state}</dd></div></dl>
    </section>
    {initial.agent.state !== "stopping" && initial.agent.state !== "stopped" && <RiskManagementSection agentId={initial.agent.id} initial={risk} />}
    <AgentSectionPage title="Telegram" description="Connect Telegram to this Agent’s main chat in a future update." />
    <DeleteAgentSection agent={initial.agent} />
  </div>;
}