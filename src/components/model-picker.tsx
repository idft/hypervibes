import { useRef, useState } from "react";
import type { ModelChoice } from "../providers/provider-service.js";
import type { ModelRef } from "@earendil-works/pi-durable";
import { ConfigDialog, configButton, configInput } from "./config-dialog.js";

export const modelKey = (model: ModelRef) => JSON.stringify([model.provider, model.modelId]);

export function ModelPicker({ choices, value, onChange, thinkingLevel, disabled = false }: {
  choices: ModelChoice[]; value: string; onChange: (value: string, thinkingLevel: string) => void | Promise<void>; thinkingLevel?: string; disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const selected = choices.find((choice) => modelKey(choice) === value);
  return <div className="space-y-2 text-sm text-zinc-300">
    <button type="button" disabled={disabled} onClick={() => setOpen(true)} aria-haspopup="dialog" aria-label="Choose model" className={`${configButton} flex w-full items-center justify-between gap-4 rounded-xl text-left`}>
      <span>{selected ? <>{selected.name}<span className="block text-xs text-zinc-500">{selected.provider} / {selected.modelId}{thinkingLevel !== undefined && <span className="text-zinc-400"> · Thinking: {thinkingLevel || "Default"}</span>}</span></> : "Select a model…"}</span><span>Choose model</span>
    </button>
    {open && <ModelDialog choices={choices} value={value} thinkingLevel={thinkingLevel} onCancel={() => setOpen(false)} onApply={async (next, thinking) => { await onChange(next, thinking); setOpen(false); }} />}
  </div>;
}

function ModelDialog({ choices, value, thinkingLevel, onCancel, onApply }: { choices: ModelChoice[]; value: string; thinkingLevel?: string; onCancel: () => void; onApply: (value: string, thinkingLevel: string) => Promise<void> }) {
  const [draft, setDraft] = useState(value);
  const [thinking, setThinking] = useState(thinkingLevel ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  async function apply() {
    if (inFlight.current) return;
    inFlight.current = true; setSaving(true); setError("");
    try { await onApply(draft, thinking); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save model. Please retry."); }
    finally { inFlight.current = false; setSaving(false); }
  }
  const selected = choices.find((choice) => modelKey(choice) === draft);
  const [search, setSearch] = useState("");
  const [provider, setProvider] = useState("");
  const matches = choices.filter((choice) => `${choice.provider} ${choice.name} ${choice.modelId}`.toLowerCase().includes(search.trim().toLowerCase()));
  const providers = [...new Set(matches.map((choice) => choice.provider))];
  const visible = matches.filter((choice) => !provider || choice.provider === provider);
  return <ConfigDialog title="Select model" onCancel={() => { if (!inFlight.current) onCancel(); }} onApply={() => { void apply(); }} applyDisabled={saving || !choices.some((choice) => modelKey(choice) === draft)}>
    <fieldset disabled={saving} className="space-y-5 disabled:opacity-60">
    <label className="block space-y-2 text-sm">Search providers and models<input autoFocus type="search" value={search} onChange={(event) => { setSearch(event.target.value); setProvider(""); }} className={configInput} placeholder="Provider, model name or ID" /></label>
    <div className="grid gap-4 sm:grid-cols-[12rem_minmax(0,1fr)]">
      <nav aria-label="Model providers" className="flex max-h-80 flex-wrap content-start gap-2 overflow-y-auto sm:flex-col">
        {["", ...providers].map((name) => <button type="button" key={name} aria-pressed={provider === name} onClick={() => setProvider(name)} className={`${configButton} rounded-lg text-left aria-pressed:border-violet-500 aria-pressed:bg-violet-950`}>{name || "All providers"}</button>)}
      </nav>
      <div className="max-h-80 space-y-2 overflow-y-auto" role="group" aria-label="Models">
        {visible.map((choice) => <label key={modelKey(choice)} className="flex cursor-pointer items-start gap-3 rounded-xl border border-zinc-800 p-3 hover:bg-zinc-900 has-checked:border-violet-500">
          <input type="radio" name="model" checked={draft === modelKey(choice)} onChange={() => { setDraft(modelKey(choice)); setThinking(""); }} className="mt-1 cursor-pointer" />
          <span className="min-w-0 break-words text-sm">{choice.name}<span className="block text-xs text-zinc-500">{choice.provider} / {choice.modelId}</span></span>
        </label>)}
        {!visible.length && <p className="text-sm text-zinc-400">No matching models. Connect a provider or try another search.</p>}
      </div>
    </div>
    {thinkingLevel !== undefined && Boolean(selected?.thinkingLevels.length) && <label className="block space-y-2 text-sm">Thinking level<select value={thinking} onChange={(event) => setThinking(event.target.value)} className={`${configInput} cursor-pointer`}><option value="">Default</option>{selected?.thinkingLevels.map((level) => <option key={level}>{level}</option>)}</select></label>}
    </fieldset>
    {saving && <p role="status" className="text-sm text-zinc-400">Saving…</p>}
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </ConfigDialog>;
}
