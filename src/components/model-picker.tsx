import type { ModelChoice } from "../providers/provider-service.js";
import type { ModelRef } from "@earendil-works/pi-durable";

export const modelKey = (model: ModelRef) => JSON.stringify([model.provider, model.modelId]);

export function ModelPicker({ choices, value, onChange, disabled = false }: {
  choices: ModelChoice[]; value: string; onChange: (value: string) => void; disabled?: boolean;
}) {
  return <label className="block space-y-2 text-sm text-zinc-300">
    <span>AI provider / model</span>
    <select aria-label="AI provider / model" value={value} onChange={(event) => onChange(event.target.value)} disabled={disabled} required className="block w-full cursor-pointer rounded-xl border border-zinc-700 bg-zinc-950 px-3 py-2 text-sm text-white disabled:opacity-50">
      <option value="">Select a model…</option>
      {[...new Set(choices.map((model) => model.provider))].map((provider) => <optgroup key={provider} label={provider}>
        {choices.filter((model) => model.provider === provider).map((model) => <option key={modelKey(model)} value={modelKey(model)}>{model.name} ({model.modelId})</option>)}
      </optgroup>)}
    </select>
  </label>;
}
