import { useRef, useState } from "react";
import type { ModelChoice } from "../providers/provider-service.js";
import type { ModelRef } from "@earendil-works/pi-durable";
import { ConfigDialog, configButton, configInput } from "./config-dialog.js";
import { ProviderLogo } from "./provider-logo.js";

export const modelKey = (model: ModelRef) => JSON.stringify([model.provider, model.modelId]);

/** Cost as a bare number string, e.g. "3" or "0.0015". */
export const costLabel = (value: number) => `${`${Number(value.toFixed(4))}`.replace(/\.0+$/, "")}`;
/** Currency-style amount with at least two decimals and no symbol, e.g. "3.00". */
export const costCurrency = (value: number) => {
  const bare = costLabel(value);
  const [whole, decimals = ""] = bare.split(".");
  return decimals.length >= 2 ? bare : `${whole}.${(decimals + "00").slice(0, 2)}`;
};
export const usd = (value: number) => `$${costLabel(value)}`;

/** Context window as a compact "200K" / "1M" token count. */
export function choiceContextTokens(choice: ModelChoice): string {
  if (choice.contextWindow >= 1_000_000) return `${choice.contextWindow / 1_000_000}M`;
  if (choice.contextWindow >= 1_000) return `${Math.round(choice.contextWindow / 1_000)}K`;
  return `${choice.contextWindow}`;
}

function choiceContext(choice: ModelChoice): string {
  return `${choiceContextTokens(choice)} ctx`;
}

export function choiceMetadata(choice: ModelChoice): string {
  const parts: string[] = [choiceContext(choice)];
  if (choice.reasoning) parts.push("reasoning");
  parts.push(`${usd(choice.inputCost)}/${usd(choice.outputCost)}`);
  return parts.join(" · ");
}

export function ModelPicker({ choices, value, onChange, thinkingLevel, disabled = false }: {
  choices: ModelChoice[]; value: string; onChange: (value: string, thinkingLevel: string) => void | Promise<void>; thinkingLevel?: string; disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const selected = choices.find((choice) => modelKey(choice) === value);
  return <div className="space-y-2 text-sm text-zinc-300">
    <button type="button" disabled={disabled} onClick={() => setOpen(true)} aria-haspopup="dialog" aria-label="Choose model" className={`${configButton} flex w-full items-center justify-between gap-4 rounded-xl text-left`}>
      <span className="flex min-w-0 items-center gap-2">{selected && <ProviderLogo provider={selected.provider} className="h-8 w-8 shrink-0 rounded-lg border border-zinc-800 bg-white p-1" />}{selected ? <span className="min-w-0 truncate">{selected.name}<span className="block text-xs text-zinc-500">{selected.providerName || selected.provider} / {selected.modelId}{thinkingLevel !== undefined && <span className="text-zinc-400"> · Thinking: {thinkingLevel || "Default"}</span>}</span></span> : "Select a model…"}</span><span>Choose model</span>
    </button>
    {open && <ModelDialog choices={choices} value={value} thinkingLevel={thinkingLevel} onCancel={() => setOpen(false)} onApply={async (next, thinking) => { await onChange(next, thinking); setOpen(false); }} />}
  </div>;
}

/** Reusable model dialog for hosts that render their own trigger. */
export function ModelDialog({ choices, value, thinkingLevel, onCancel, onApply }: { choices: ModelChoice[]; value: string; thinkingLevel?: string; onCancel: () => void; onApply: (value: string, thinkingLevel: string) => Promise<void> }) {
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
  const providers = [...new Set(matches.map((choice) => choice.provider))].map((id) => ({ id, name: matches.find((choice) => choice.provider === id)!.providerName || id }));
  const visible = matches.filter((choice) => !provider || choice.provider === provider);
  return <ConfigDialog title="Select model" onCancel={() => { if (!inFlight.current) onCancel(); }} onApply={() => { void apply(); }} applyDisabled={saving || !choices.some((choice) => modelKey(choice) === draft)}>
    <fieldset disabled={saving} className="space-y-5 disabled:opacity-60">
    <label className="block space-y-2 text-sm">Search providers and models<input autoFocus type="search" value={search} onChange={(event) => { setSearch(event.target.value); setProvider(""); }} className={configInput} placeholder="Provider, model name or ID" /></label>
    <div className="grid gap-4 sm:grid-cols-[12rem_minmax(0,1fr)]">
      <nav aria-label="Model providers" className="flex max-h-40 flex-wrap content-start gap-2 overflow-y-auto sm:max-h-80 sm:flex-col">
        {[{ id: "", name: "" }, ...providers].map((entry) => <button type="button" key={entry.id} aria-pressed={provider === entry.id} onClick={() => setProvider(entry.id)} className={`${configButton} flex items-center gap-2 rounded-lg text-left aria-pressed:border-violet-500 aria-pressed:bg-violet-950`}>{entry.id ? <ProviderLogo provider={entry.id} className="h-5 w-5 shrink-0 rounded-sm bg-white p-0.5" /> : null}{entry.name || "All providers"}</button>)}
      </nav>
      <div className="h-[28rem] space-y-2 overflow-y-auto" role="group" aria-label="Models">
        {visible.map((choice) => <label key={modelKey(choice)} className="flex cursor-pointer items-center gap-3 rounded-xl border border-zinc-800 p-3 hover:bg-zinc-900 has-checked:border-violet-500 has-checked:bg-violet-950/40 has-focus-visible:ring-2 has-focus-visible:ring-violet-400">
          <input type="radio" name="model" checked={draft === modelKey(choice)} onChange={() => { setDraft(modelKey(choice)); setThinking(""); }} className="h-0 w-0 cursor-pointer" />
          <ProviderLogo provider={choice.provider} className="h-9 w-9 shrink-0 rounded-lg border border-zinc-800 bg-white p-1" />
          <span className="min-w-0 break-words text-sm">{choice.name}<span className="block text-xs text-zinc-500">{choice.modelId}<span className="text-zinc-600"> · {choiceMetadata(choice)}</span></span></span>
        </label>)}
        {!visible.length && <p className="text-sm text-zinc-400">No matching models. Connect a provider or try another search.</p>}
      </div>
    </div>
    {thinkingLevel !== undefined && Boolean(selected?.thinkingLevels.some((level) => level !== "off")) && <label className="block space-y-2 text-sm">Thinking level<select value={thinking} onChange={(event) => setThinking(event.target.value)} className={`${configInput} cursor-pointer`}><option value="">Default</option>{selected?.thinkingLevels.filter((level) => level !== "off").map((level) => <option key={level}>{level}</option>)}</select></label>}
    </fieldset>
    {saving && <p role="status" className="text-sm text-zinc-400">Saving…</p>}
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </ConfigDialog>;
}
