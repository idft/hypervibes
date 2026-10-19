import { useRouter } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { toast } from "sonner";
import type { SubagentInput } from "../agents/subagent-input.js";
import { subagentTemplates, templateDraft } from "../agents/subagent-templates.js";
import { saveSubagent } from "../web/subagents.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { ModelPicker, modelKey } from "./model-picker.js";

const inputClass = "mt-2 block w-full rounded-lg border border-zinc-700 bg-zinc-950 p-3 text-sm text-white";
const blank: Omit<SubagentInput, "provider" | "modelId" | "thinkingLevel"> = { name: "", description: "", instructions: "Complete the assigned task using available tools. Cite evidence and timestamps, distinguish facts from assumptions, and report missing or stale data. Do not invent access to unavailable tools.", tools: [], enabled: false, delegatable: true, persistent: false, schedule: "", delaySeconds: 30, completion: "wake", timeoutSeconds: 900 };
type Model = Pick<SubagentInput, "provider" | "modelId" | "thinkingLevel">;

export function NewSubagentPage({ agentId, model, choices }: { agentId: string; model: Model; choices: ModelChoice[] }) {
  const router = useRouter();
  const [name, setName] = useState(""), [description, setDescription] = useState("");
  const [templateId, setTemplateId] = useState("");
  const [selectedModel, setSelectedModel] = useState<Model>(() => {
    const choice = choices.find((entry) => modelKey(entry) === modelKey(model));
    return { ...model, thinkingLevel: choice?.thinkingLevels.includes(model.thinkingLevel) ? model.thinkingLevel : "" };
  });
  const selected = choices.find((choice) => modelKey(choice) === modelKey(selectedModel));
  const [pending, setPending] = useState(false), [error, setError] = useState("");
  const inFlight = useRef(false), requestId = useRef<string | null>(null), createdId = useRef<string | null>(null);
  function changeTemplate(id: string) {
    setTemplateId(id);
    const template = subagentTemplates.find((entry) => entry.id === id);
    setName(template?.name ?? "");
    setDescription(template?.description ?? "");
  }
  async function create() {
    if (inFlight.current || !name.trim() || !selected) return;
    inFlight.current = true; setPending(true); setError("");
    try {
      if (!createdId.current) {
        requestId.current ??= crypto.randomUUID();
        const settings = { ...(templateId ? templateDraft(templateId, selectedModel) : { ...blank, ...selectedModel }), name, description, enabled: false };
        const result = await saveSubagent({ data: { agentId, id: "", requestId: requestId.current, templateId, settings } });
        if (!result.ok) { requestId.current = null; setError(result.error); return; }
        createdId.current = result.data.id;
        toast.success("Sub-agent created paused", { description: "Review the configuration, then enable when ready." });
      }
      await router.navigate({ to: "/agents/$agentId/sub-agents/$subagentId", params: { agentId, subagentId: createdId.current } });
    } catch { setError("Unable to finish. Try Create again to retry safely without creating a duplicate."); }
    finally { inFlight.current = false; setPending(false); }
  }
  return <section className="space-y-6 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <header><h2 className="text-lg font-medium text-white">New sub-agent</h2></header>
    <form onSubmit={(event) => { event.preventDefault(); void create(); }}>
      <div className="grid gap-6 md:grid-cols-[minmax(12rem,1fr)_minmax(0,2fr)]">
        <fieldset disabled={pending || requestId.current !== null} className="min-w-0 space-y-3 disabled:opacity-60">
          <legend className="mb-3 text-sm font-medium text-zinc-300">Templates</legend>
          <div role="group" aria-label="Sub-agent templates" className="space-y-2">
            {subagentTemplates.map((entry) => <button key={entry.id} type="button" aria-pressed={templateId === entry.id} onClick={() => changeTemplate(entry.id)} className="block w-full cursor-pointer rounded-xl border border-zinc-800 p-4 text-left text-sm text-zinc-300 hover:bg-zinc-900 aria-pressed:border-violet-500 aria-pressed:bg-violet-950/40 aria-pressed:text-white disabled:cursor-not-allowed">{entry.name}</button>)}
          </div>
          <div className="border-t border-zinc-800 pt-3"><button type="button" aria-pressed={templateId === ""} onClick={() => changeTemplate("")} className="block w-full cursor-pointer rounded-xl border border-zinc-800 p-4 text-left text-sm text-zinc-300 hover:bg-zinc-900 aria-pressed:border-violet-500 aria-pressed:bg-violet-950/40 aria-pressed:text-white disabled:cursor-not-allowed">Custom</button></div>
        </fieldset>
        <div className="min-w-0 space-y-4 md:border-l md:border-zinc-800 md:pl-6">
          <fieldset disabled={pending || requestId.current !== null} className="space-y-4 disabled:opacity-60">
            <label className="block text-sm text-zinc-300">Name<input required maxLength={80} value={name} onChange={(event) => setName(event.target.value)} className={inputClass} /></label>
            <label className="block text-sm text-zinc-300">Description (optional)<textarea rows={3} maxLength={500} value={description} onChange={(event) => setDescription(event.target.value)} className={inputClass} /></label>
            <div className="space-y-2"><h3 className="text-sm text-zinc-300">Model</h3>
              {!selected && <p role="status" className="text-sm text-amber-200">The root Agent’s model ({selectedModel.provider}/{selectedModel.modelId}) is unavailable. Choose an available model or reconnect its provider.</p>}
              <ModelPicker choices={choices} value={modelKey(selectedModel)} thinkingLevel={selectedModel.thinkingLevel} disabled={pending || requestId.current !== null} onChange={(value, thinkingLevel) => { const choice = choices.find((entry) => modelKey(entry) === value); if (choice) setSelectedModel({ provider: choice.provider, modelId: choice.modelId, thinkingLevel }); }} />
            </div>
          </fieldset>
          <p className="text-sm text-zinc-400">Created paused. Review instructions, tools and scheduling on the detail page before enabling.</p>
          {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
          <button disabled={pending || !name.trim() || !selected} className="cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50">{pending ? "Creating…" : "Create sub-agent"}</button>
        </div>
      </div>
    </form>
  </section>;
}
