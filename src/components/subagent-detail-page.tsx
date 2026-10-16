import { useRouter } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { toast } from "sonner";
import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { candleIntervals, defaultMainInstructions, mainToolNames, workerToolNames, type SubagentInput } from "../agents/subagent-input.js";
import { archiveSubagent, getSubagent, runSubagent, saveSubagent, stopSubagentRun } from "../web/subagents.js";
import { ModelPicker, modelKey } from "./model-picker.js";
import { ArchiveSubagentDialog } from "./archive-subagent-dialog.js";

const panel = "space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6";
const inputClass = "mt-2 block w-full rounded-lg border border-zinc-700 bg-zinc-950 p-3 text-sm text-white";
const button = "cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50";
const defaults: SubagentInput = { name: "", description: "", instructions: "Complete the assigned task using available tools. Cite evidence and timestamps, distinguish facts from assumptions, and report missing or stale data. Do not invent access to unavailable tools.", provider: "", modelId: "", thinkingLevel: "", tools: [], enabled: true, delegatable: true, persistent: false, schedule: "", delaySeconds: 30, completion: "wake", timeoutSeconds: 900 };
export function SubagentDetailPage({ agentId, initial, choices }: { agentId: string; initial?: { definition: SubagentRow; runs: SubagentRun[]; schedulerError: string | null }; choices: ModelChoice[] }) {
  const router = useRouter();
  const [definition, setDefinition] = useState(initial?.definition);
  const [form, setForm] = useState<SubagentInput>(initial?.definition ?? { ...defaults, provider: choices[0]?.provider ?? "", modelId: choices[0]?.modelId ?? "" });
  const [runs, setRuns] = useState(initial?.runs ?? []);
  const [schedulerError, setSchedulerError] = useState(initial?.schedulerError);
  const [task, setTask] = useState("");
  const [pending, setPending] = useState(false), [error, setError] = useState("");
  const [archiveOpen, setArchiveOpen] = useState(false);
  const inFlight = useRef(false), creationId = useRef<string | null>(null), assignmentId = useRef<{ task: string; id: string } | null>(null);
  const main = definition?.kind === "orchestrator";
  const selected = choices.find((choice) => modelKey(choice) === modelKey(form));
  const set = <K extends keyof SubagentInput>(key: K, value: SubagentInput[K]) => setForm((current) => ({ ...current, [key]: value }));
  async function refresh() {
    if (!definition) return;
    const response = await getSubagent({ data: { agentId, id: definition.id } });
    if (!response.ok) throw new Error(response.error);
    setRuns(response.data.runs); setSchedulerError(response.data.schedulerError);
  }
  async function perform(action: "save" | "run" | "archive" | "refresh" | "stop", runId = "") {
    if (inFlight.current) return;
    inFlight.current = true; setPending(true); setError("");
    try {
      if (action === "save") {
        creationId.current ??= crypto.randomUUID();
        const result = await saveSubagent({ data: { agentId, id: definition?.id ?? "", requestId: creationId.current, settings: form } });
        if (!result.ok) { setError(result.error); return; }
        toast.success(definition ? "Sub-agent updated" : "Sub-agent created");
        setDefinition(result.data); setForm(result.data);
        return;
      }
      if (action === "refresh") { await refresh(); return; }
      if (action === "run") {
        if (assignmentId.current?.task !== task) assignmentId.current = { task, id: crypto.randomUUID() };
        const result = await runSubagent({ data: { agentId, id: definition!.id, task, requestId: assignmentId.current!.id } });
        if (!result.ok) { setError(result.error); return; }
        assignmentId.current = null; toast.success("Assignment started", { description: "Refresh run history for its outcome." });
      } else if (action === "stop") {
        const result = await stopSubagentRun({ data: { agentId, runId } });
        if (!result.ok) { setError(result.error); return; }
        toast.success("Assignment stopped", { description: "Existing exchange orders were not cancelled." });
      } else {
        const result = await archiveSubagent({ data: { agentId, id: definition!.id } });
        if (!result.ok) { setError(result.error); return; }
        setDefinition((current) => current ? { ...current, archived: true, enabled: false } : current); setForm((current) => ({ ...current, enabled: false })); toast.success("Sub-agent archived", { description: "History is preserved." });
        await router.invalidate();
        await router.navigate({ to: "/agents/$agentId/sub-agents", params: { agentId } });
        return;
      }
      await refresh();
    } catch { setError("Unable to complete the operation. Reload to check current state; retrying an assignment uses the same request ID."); }
    finally { inFlight.current = false; setPending(false); }
  }
  const toggle = (key: "enabled" | "delegatable" | "persistent", label: string) => <label className="flex items-center gap-3 text-sm text-zinc-300"><input type="checkbox" checked={form[key]} onChange={(event) => set(key, event.target.checked)} disabled={pending || definition?.archived} className="cursor-pointer" />{label}</label>;
  return <div className="w-full space-y-3">
    <form onSubmit={(event) => { event.preventDefault(); void perform("save"); }} className={panel}>
      <h2 className="text-lg font-medium text-white">{definition ? main ? "Orchestrator · Main conversation" : definition.name : "Create subagent"}</h2>
      {main && <p className="text-sm text-zinc-400">This is the existing main chat. It cannot be disabled, archived, or deleted.</p>}
      {definition?.archived && <p className="text-sm text-amber-200">Archived — new assignments are disabled. History remains available.</p>}
      <fieldset disabled={pending || definition?.archived} className="space-y-4 disabled:opacity-60">
        <label className="block text-sm text-zinc-300">Name<input required maxLength={80} value={form.name} onChange={(event) => set("name", event.target.value)} className={inputClass} /></label>
        <label className="block text-sm text-zinc-300">Description<input maxLength={500} value={form.description} onChange={(event) => set("description", event.target.value)} className={inputClass} /></label>
        <label className="block text-sm text-zinc-300">Instructions<textarea required rows={7} maxLength={32000} value={form.instructions} onChange={(event) => set("instructions", event.target.value)} className={inputClass} placeholder={main ? defaultMainInstructions : defaults.instructions} /></label>
        <h3 className="font-medium text-white">Model & thinking</h3>
        {definition && !selected && <p className="text-sm text-amber-200">Current model: {form.provider}/{form.modelId}. Reconnect its provider or select an available model.</p>}
        <ModelPicker choices={choices} value={selected ? modelKey(form) : ""} onChange={(value) => { const choice = choices.find((entry) => modelKey(entry) === value); if (choice) setForm((current) => ({ ...current, provider: choice.provider, modelId: choice.modelId, thinkingLevel: "" })); }} disabled={pending} />
        <a href="/providers" className="inline-block cursor-pointer text-sm text-violet-300 hover:underline">Manage providers</a>
        {Boolean(selected?.thinkingLevels.length) && <label className="block text-sm text-zinc-300">Thinking level<select value={form.thinkingLevel} onChange={(event) => set("thinkingLevel", event.target.value)} className={inputClass}><option value="">Default</option>{selected?.thinkingLevels.map((level) => <option key={level}>{level}</option>)}</select></label>}
        <h3 className="font-medium text-white">Tools</h3>
        <div className="grid gap-2 sm:grid-cols-2">{(main ? mainToolNames : workerToolNames).map((name) => <label key={name} className="flex items-center gap-2 font-mono text-xs text-zinc-300"><input type="checkbox" checked={form.tools.includes(name)} onChange={(event) => set("tools", event.target.checked ? [...form.tools, name] : form.tools.filter((tool) => tool !== name))} className="cursor-pointer" />{name}</label>)}</div>
        {!main && <>
          <h3 className="font-medium text-white">Assignments & context</h3>
          {toggle("enabled", "Subagent enabled")}{toggle("delegatable", "Allow orchestrator delegation")}{toggle("persistent", "Reuse a persistent conversation (otherwise fresh context per assignment)")}
          <label className="block text-sm text-zinc-300">Assignment timeout (seconds)<input type="number" min={30} max={3600} value={form.timeoutSeconds} onChange={(event) => set("timeoutSeconds", Number(event.target.value))} className={inputClass} /></label>
          <h3 className="font-medium text-white">Schedule</h3>
          <label className="block text-sm text-zinc-300">UTC candle close<select value={form.schedule} onChange={(event) => set("schedule", event.target.value)} className={`${inputClass} cursor-pointer`}><option value="">No schedule</option>{Object.keys(candleIntervals).map((interval) => <option key={interval} value={interval}>{interval === "1d" ? "1d / every 24 hours" : interval}</option>)}</select></label>
          {form.schedule && <label className="block text-sm text-zinc-300">Delay after close (seconds)<input type="number" min={0} max={300} value={form.delaySeconds} onChange={(event) => set("delaySeconds", Number(event.target.value))} className={inputClass} /></label>}
          <label className="block text-sm text-zinc-300">Scheduled / manual completion<select value={form.completion} onChange={(event) => set("completion", event.target.value as SubagentInput["completion"])} className={`${inputClass} cursor-pointer`}><option value="save">Save only</option><option value="notify">Notify orchestrator without a model call</option><option value="wake">Notify and wake orchestrator</option></select></label>
        </>}
        <div className="flex items-center gap-3">
          <button disabled={pending || !selected} className={button}>{pending ? "Saving…" : definition ? "Save settings" : "Create subagent"}</button>
          <a href={`/agents/${agentId}/sub-agents`} className="cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-800">Cancel</a>
        </div>
      </fieldset>
    </form>
    {definition && !main && <section className={panel}><h3 className="font-medium text-white">Run manually</h3><label className="block text-sm text-zinc-300">Assignment<textarea rows={3} maxLength={16000} value={task} onChange={(event) => setTask(event.target.value)} className={inputClass} /></label><div className="flex gap-3"><button disabled={pending || !task.trim() || !definition.enabled || definition.archived} onClick={() => { void perform("run"); }} className={button}>Run now</button><button disabled={pending || definition.archived} onClick={() => setArchiveOpen(true)} className="cursor-pointer rounded-full border border-red-900 px-4 py-2 text-sm text-red-300 disabled:cursor-not-allowed disabled:opacity-50">Archive worker</button></div></section>}
    {definition && !main && <section className={panel}><header className="flex items-center justify-between"><h3 className="font-medium text-white">Run history · latest 50</h3><button disabled={pending} onClick={() => { void perform("refresh"); }} className={button}>Refresh</button></header>{schedulerError && <p role="alert" className="text-sm text-red-300">{schedulerError}</p>}{runs.length === 0 && <p className="text-sm text-zinc-500">No assignments yet.</p>}{runs.map((run) => <article key={run.id} className="space-y-2 rounded-xl border border-zinc-800 p-4"><p className="text-sm text-zinc-300">{run.status} · {run.source} · {new Date(run.createdAt).toISOString()}{(run.status === "queued" || run.status === "running") && <button disabled={pending} onClick={() => { void perform("stop", run.id); }} className="ml-4 cursor-pointer text-red-300 hover:underline">Stop assignment</button>}</p><p className="font-mono text-xs text-zinc-500">{run.id}</p><p className="whitespace-pre-wrap break-words text-sm text-zinc-300">{run.task}</p>{run.result && <details><summary className="cursor-pointer text-sm text-violet-300">Result</summary><pre className="mt-2 max-h-96 overflow-auto whitespace-pre-wrap break-words text-sm text-zinc-300">{run.result}</pre></details>}</article>)}</section>}
    {archiveOpen && definition && !main && <ArchiveSubagentDialog name={definition.name} onCancel={() => setArchiveOpen(false)} onConfirm={() => { setArchiveOpen(false); void perform("archive"); }} />}
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </div>;
}
