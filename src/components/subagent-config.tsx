import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { format } from "timeago.js";
import { candleIntervals, hyperliquidToolNames, researchToolNames, taToolNames, type AssignmentSettings, type SubagentInput } from "../agents/subagent-input.js";
import { memoryToolNames } from "../memory/memory-contracts.js";
import { ConfigDialog, configButton, configInput } from "./config-dialog.js";

import { SubagentEnabledToggle } from "./subagent-enabled-toggle.js";

type Identity = Pick<SubagentInput, "name" | "description">;
export function SubagentIdentity({ value, main, agentId, enabled, onToggleEnabled, onChange, actions }: { actions?: ReactNode; value: Identity; main: boolean; agentId: string; enabled: boolean; onToggleEnabled: () => void; onChange: (value: Identity) => Promise<void> }) {
  const [open, setOpen] = useState(false);
  return <section className="space-y-2">
    <header className="flex flex-wrap items-center justify-between gap-3">
        <nav aria-label="Sub-agent breadcrumb" className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <a href={`/agents/${agentId}/sub-agents`} className="cursor-pointer text-sm text-zinc-400 hover:text-white"><span aria-hidden="true">← </span>Sub-agents</a>
          <span aria-hidden="true" className="text-zinc-600">/</span>
          <h2 aria-current="page" className="break-words text-lg font-medium text-white">{value.name || "Create subagent"}{main && " · Main conversation"}</h2>
          {!main && <SubagentEnabledToggle enabled={enabled} onClick={onToggleEnabled} />}
          <span className="min-w-0 break-words text-sm text-zinc-400">{value.description || "No description"}</span>
          <button type="button" aria-label="Edit name and description" title="Edit name and description" aria-haspopup="dialog" onClick={() => setOpen(true)} className="inline-flex shrink-0 cursor-pointer items-center justify-center rounded-md p-1 text-zinc-400 hover:bg-zinc-800 hover:text-white">
            <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" className="size-4"><path d="m16 4 4 4M4 20l4-1L21 6a2.8 2.8 0 0 0-4-4L4 15z" /></svg>
          </button>
        </nav>
        {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
    <hr className="border-zinc-800" />
    {open && <IdentityDialog value={value} onCancel={() => setOpen(false)} onApply={async (next) => { await onChange(next); setOpen(false); }} />}
  </section>;
}

function IdentityDialog({ value, onCancel, onApply }: { value: Identity; onCancel: () => void; onApply: (value: Identity) => Promise<void> }) {
  const [draft, setDraft] = useState(value);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const descriptionHintId = useId();
  async function apply() {
    if (inFlight.current || !draft.name.trim()) return;
    inFlight.current = true; setSaving(true); setError("");
    try { await onApply({ name: draft.name.trim(), description: draft.description.trim() }); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save name and description. Please retry."); }
    finally { inFlight.current = false; setSaving(false); }
  }
  return <ConfigDialog title="Edit name and description" onCancel={() => { if (!inFlight.current) onCancel(); }} onApply={() => { void apply(); }} applyDisabled={saving || !draft.name.trim()}>
    <fieldset disabled={saving} className="space-y-5 disabled:opacity-60">
      <label className="block space-y-2 text-sm">Name<input autoFocus required maxLength={80} value={draft.name} onChange={(event) => setDraft((current) => ({ ...current, name: event.target.value }))} className={configInput} /></label>
      <label className="block space-y-2 text-sm">Description<input aria-describedby={descriptionHintId} maxLength={500} value={draft.description} onChange={(event) => setDraft((current) => ({ ...current, description: event.target.value }))} className={configInput} /><span id={descriptionHintId} className="block text-xs text-zinc-400">Human-readable description.</span></label>
    </fieldset>
    {saving && <p role="status" className="text-sm text-zinc-400">Saving…</p>}
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </ConfigDialog>;
}

const toolGroups = [
  { name: "Hyperliquid", tools: hyperliquidToolNames },
  { name: "Research · Code Mode & Artifacts", tools: researchToolNames },
  { name: "Technical analysis · TA", tools: taToolNames },
  { name: "Memories & Insights", tools: memoryToolNames },
  { name: "HyperVibes · Subagents", tools: ["list_subagents", "delegate_subagent", "get_subagent_run"] },
];

export function ToolPicker({ tools, main, onChange }: { tools: string[]; main: boolean; onChange: (tools: string[]) => Promise<void> }) {
  const [open, setOpen] = useState(false);
  return <section className="space-y-2">
    <header className="flex items-center justify-between gap-4"><h3 className="font-medium text-white">Tools</h3><button type="button" aria-haspopup="dialog" onClick={() => setOpen(true)} className={configButton}>Edit tools</button></header>
    <p className="text-sm text-zinc-400">{tools.length ? `${tools.length} ${tools.length === 1 ? "tool" : "tools"} allowed` : "No tools allowed"}</p>
    {toolGroups.map((group) => {
      const enabled = group.tools.filter((name) => tools.includes(name));
      if (!enabled.length) return null;
      const summary = `${group.name}: ${enabled.join(", ")}`;
      return <p key={group.name} className="line-clamp-2 break-words text-sm text-zinc-200" title={summary}>{group.name}: <span className="font-mono">{enabled.join(", ")}</span></p>;
    })}
    {open && <ToolDialog tools={tools} main={main} onCancel={() => setOpen(false)} onApply={async (next) => { await onChange(next); setOpen(false); }} />}
  </section>;
}

function ToolDialog({ tools, main, onCancel, onApply }: { tools: string[]; main: boolean; onCancel: () => void; onApply: (tools: string[]) => Promise<void> }) {
  const [draft, setDraft] = useState(tools);
  const [search, setSearch] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  async function apply() {
    if (inFlight.current) return;
    inFlight.current = true; setSaving(true); setError("");
    try { await onApply(draft); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save tools. Please retry."); }
    finally { inFlight.current = false; setSaving(false); }
  }
  const groups = toolGroups.filter((group) => main || group.name !== "HyperVibes · Subagents").map((group) => ({ ...group, visibleTools: group.tools.filter((name) => `${group.name} ${name}`.toLowerCase().includes(search.trim().toLowerCase())) })).filter((group) => group.visibleTools.length);
  return <ConfigDialog title="Allowed tools" onCancel={() => { if (!inFlight.current) onCancel(); }} onApply={() => { void apply(); }} applyDisabled={saving}>
    <fieldset disabled={saving} className="space-y-4 disabled:opacity-60">
    <label className="block space-y-2 text-sm">Search extensions and tools<input autoFocus type="search" value={search} onChange={(event) => setSearch(event.target.value)} className={configInput} /></label>
    <div className="max-h-80 space-y-4 overflow-y-auto">
      {groups.map((group) => <fieldset key={group.name} className="space-y-3 rounded-xl border border-zinc-800 p-4"><legend className="px-2 text-sm font-medium text-white"><span className="inline-flex flex-wrap items-center gap-3">
          <span>{group.name}</span>
          <span className="inline-flex items-center gap-2 text-xs font-normal">
            <button type="button" aria-label={`All tools in ${group.name}`} onClick={() => setDraft((current) => [...new Set([...current, ...group.tools])])} className="cursor-pointer text-zinc-400 hover:text-violet-300 hover:underline">All</button>
            <span aria-hidden="true" className="text-zinc-700">/</span>
            <button type="button" aria-label={`No tools in ${group.name}`} onClick={() => setDraft((current) => current.filter((tool) => !group.tools.some((name) => name === tool)))} className="cursor-pointer text-zinc-400 hover:text-violet-300 hover:underline">None</button>
          </span>
        </span></legend>
        <div className="grid gap-3 sm:grid-cols-2">{group.visibleTools.map((name) => <label key={name} className="flex cursor-pointer items-center gap-3 font-mono text-xs"><input type="checkbox" checked={draft.includes(name)} onChange={(event) => setDraft((current) => event.target.checked ? [...current, name] : current.filter((tool) => tool !== name))} className="cursor-pointer" />{name}</label>)}</div>
      </fieldset>)}
      {!groups.length && <p className="text-sm text-zinc-400">No matching tools.</p>}
    </div>
    <p className="text-sm text-zinc-400">{draft.length} selected{saving ? " · Saving…" : ""}</p>
    </fieldset>
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </ConfigDialog>;
}

export function AssignmentConfig({ value, onChange }: { value: AssignmentSettings; onChange: (value: AssignmentSettings) => Promise<void> }) {
  const [open, setOpen] = useState(false);
  return <section className="space-y-2">
    <header className="flex items-center justify-between gap-4"><h3 className="font-medium text-white">Assignments & context</h3><button type="button" aria-haspopup="dialog" onClick={() => setOpen(true)} className={configButton}>Edit assignments</button></header>
    <p className="text-sm text-zinc-200">Orchestrator delegation {value.delegatable ? "allowed" : "disabled"}</p>
    <p className="text-sm text-zinc-200">{value.persistent ? "Persistent conversation" : "Fresh context per assignment"} · Timeout: {value.timeoutSeconds}s</p>
    {open && <AssignmentDialog value={value} onCancel={() => setOpen(false)} onApply={async (next) => { await onChange(next); setOpen(false); }} />}
  </section>;
}

function AssignmentDialog({ value, onCancel, onApply }: { value: AssignmentSettings; onCancel: () => void; onApply: (value: AssignmentSettings) => Promise<void> }) {
  const [draft, setDraft] = useState<AssignmentSettings>({ delegatable: value.delegatable, persistent: value.persistent, timeoutSeconds: value.timeoutSeconds });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const valid = Number.isInteger(draft.timeoutSeconds) && draft.timeoutSeconds >= 30 && draft.timeoutSeconds <= 3600;
  async function apply() {
    if (inFlight.current || !valid) return;
    inFlight.current = true; setSaving(true); setError("");
    try { await onApply(draft); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save assignment settings. Please retry."); }
    finally { inFlight.current = false; setSaving(false); }
  }
  const toggle = (key: "delegatable" | "persistent", label: string) => <label className="flex cursor-pointer items-center gap-3 text-sm"><input type="checkbox" checked={draft[key]} onChange={(event) => setDraft((current) => ({ ...current, [key]: event.target.checked }))} className="cursor-pointer" />{label}</label>;
  return <ConfigDialog title="Assignments & context" onCancel={() => { if (!inFlight.current) onCancel(); }} onApply={() => { void apply(); }} applyDisabled={saving || !valid}>
    <fieldset disabled={saving} className="space-y-4 disabled:opacity-60">
      {toggle("delegatable", "Allow orchestrator delegation")}
      {toggle("persistent", "Reuse a persistent conversation (otherwise fresh context per assignment)")}
      <label className="block space-y-2 text-sm">Assignment timeout (seconds)<input required type="number" min={30} max={3600} step={1} value={draft.timeoutSeconds} onChange={(event) => setDraft((current) => ({ ...current, timeoutSeconds: Number(event.target.value) }))} className={configInput} /></label>
    </fieldset>
    {saving && <p role="status" className="text-sm text-zinc-400">Saving…</p>}
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </ConfigDialog>;
}

type Schedule = Pick<SubagentInput, "schedule" | "delaySeconds" | "completion">;
const completionLabels = { save: "Save only", notify: "Notify orchestrator without a model call", wake: "Notify and wake orchestrator" } as const;
function NextScheduledRun({ timestamp }: { timestamp: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => tick((value) => value + 1), 1000);
    return () => clearInterval(timer);
  }, []);
  const exact = new Date(timestamp).toISOString();
  return <p className="text-sm text-violet-300">Next run <time dateTime={exact} title={exact} suppressHydrationWarning>{timestamp > Date.now() ? format(timestamp) : "due now"}</time></p>;
}

export function ScheduleConfig({ value, nextRunAt, onChange }: { value: Schedule; nextRunAt?: number | null; onChange: (value: Schedule) => Promise<void> }) {
  const [open, setOpen] = useState(false);
  return <section className="space-y-2">
    <header className="flex items-center justify-between gap-4"><h3 className="font-medium text-white">Schedule</h3><button type="button" aria-haspopup="dialog" onClick={() => setOpen(true)} className={configButton}>Edit schedule</button></header>
    <p className="text-sm text-zinc-400">{value.schedule ? `Every ${value.schedule} candle close (UTC) · ${value.delaySeconds}s delay` : "No schedule · manual or delegated assignments only"}</p>
    {value.schedule && nextRunAt != null && <NextScheduledRun timestamp={nextRunAt} />}
    <p className="text-xs text-zinc-500">Scheduled / manual completion: {completionLabels[value.completion]}</p>
    {open && <ScheduleDialog value={value} onCancel={() => setOpen(false)} onApply={async (next) => { await onChange(next); setOpen(false); }} />}
  </section>;
}

function ScheduleDialog({ value, onCancel, onApply }: { value: Schedule; onCancel: () => void; onApply: (value: Schedule) => Promise<void> }) {
  const [draft, setDraft] = useState(() => value.schedule ? value : { ...value, delaySeconds: 1 });
  const interval = candleIntervals[draft.schedule as keyof typeof candleIntervals];
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const valid = !draft.schedule || (Number.isInteger(draft.delaySeconds) && draft.delaySeconds >= 0 && draft.delaySeconds <= 300 && draft.delaySeconds < interval);
  async function apply() {
    if (inFlight.current || !valid) return;
    inFlight.current = true; setSaving(true); setError("");
    try { await onApply({ schedule: draft.schedule, delaySeconds: draft.delaySeconds, completion: draft.completion }); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save schedule. Please retry."); }
    finally { inFlight.current = false; setSaving(false); }
  }
  return <ConfigDialog title="Configure schedule" onCancel={() => { if (!inFlight.current) onCancel(); }} onApply={() => { void apply(); }} applyDisabled={saving || !valid}>
    <fieldset disabled={saving} className="space-y-5 disabled:opacity-60">
    <p className="text-sm text-zinc-400">Run after a UTC candle closes.</p>
    <label className="block space-y-2 text-sm">UTC candle close<select value={draft.schedule} onChange={(event) => setDraft((current) => ({ ...current, schedule: event.target.value }))} className={`${configInput} cursor-pointer`}><option value="">No schedule</option>{Object.keys(candleIntervals).map((interval) => <option key={interval} value={interval}>{interval === "1d" ? "1d / every 24 hours" : interval}</option>)}</select></label>
    {draft.schedule && <label className="block space-y-2 text-sm">Delay after candle close (seconds)<input required type="number" min={0} max={Math.min(300, interval - 1)} step={1} value={draft.delaySeconds} onChange={(event) => setDraft((current) => ({ ...current, delaySeconds: Number(event.target.value) }))} className={configInput} /></label>}
    <label className="block space-y-2 text-sm">Scheduled / manual completion<select value={draft.completion} onChange={(event) => setDraft((current) => ({ ...current, completion: event.target.value as Schedule["completion"] }))} className={`${configInput} cursor-pointer`}>{Object.entries(completionLabels).map(([key, label]) => <option key={key} value={key}>{label}</option>)}</select></label>
    </fieldset>
    {saving && <p role="status" className="text-sm text-zinc-400">Saving…</p>}
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </ConfigDialog>;
}
