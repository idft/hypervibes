import { useRouter } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { toast } from "sonner";
import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { defaultMainInstructions, type AssignmentSettings, type SubagentInput, type SubagentSections } from "../agents/subagent-input.js";
import { archiveSubagent, getSubagent, runSubagent, saveSubagent, saveSubagentAssignments, saveSubagentIdentity, saveSubagentSection, saveSubagentTools, stopSubagentRun } from "../web/subagents.js";
import { ModelPicker, modelKey } from "./model-picker.js";
import { RunSubagentDialog } from "./run-subagent-dialog.js";
import { ArchiveSubagentDialog } from "./archive-subagent-dialog.js";
import { AssignmentConfig, ScheduleConfig, SubagentIdentity, ToolPicker } from "./subagent-config.js";

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
  const [runOpen, setRunOpen] = useState(false);
  const inFlight = useRef(false), creationId = useRef<string | null>(null), assignmentId = useRef<{ task: string; id: string } | null>(null);
  const main = definition?.kind === "orchestrator";
  const selected = choices.find((choice) => modelKey(choice) === modelKey(form));
  const set = <K extends keyof SubagentInput>(key: K, value: SubagentInput[K]) => setForm((current) => ({ ...current, [key]: value }));
  const instructionsChanged = definition !== undefined && form.instructions !== definition.instructions;
  async function saveSection<K extends keyof SubagentSections>(section: K, settings: SubagentSections[K]) {
    if (!definition) { setForm((current) => ({ ...current, ...settings })); return; }
    if (inFlight.current || definition.archived) throw new Error("Settings cannot be changed right now. Please retry.");
    inFlight.current = true; setPending(true);
    try {
      const result = await saveSubagentSection({ data: { agentId, id: definition.id, section, settings } });
      if (!result.ok) throw new Error(result.error);
      const row = result.data;
      const saved = section === "enabled" ? { enabled: row.enabled } : section === "instructions" ? { instructions: row.instructions }
        : section === "model" ? { provider: row.provider, modelId: row.modelId, thinkingLevel: row.thinkingLevel }
        : { schedule: row.schedule, delaySeconds: row.delaySeconds, completion: row.completion };
      setDefinition(row); setForm((current) => ({ ...current, ...saved }));
      toast.success(section === "enabled" ? row.enabled ? "Sub-agent enabled" : "Sub-agent paused" : section === "instructions" ? "Instructions updated" : section === "model" ? "Model updated" : "Schedule updated");
    } finally { inFlight.current = false; setPending(false); }
  }
  async function toggleEnabled() {
    setError("");
    try { await saveSection("enabled", { enabled: !form.enabled }); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to change enabled status. Please retry."); }
  }
  async function saveInstructions() {
    setError("");
    try { await saveSection("instructions", { instructions: form.instructions }); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to save instructions. Please retry."); }
  }
  async function saveAssignments(settings: AssignmentSettings) {
    if (!definition) { setForm((current) => ({ ...current, ...settings })); return; }
    if (inFlight.current || definition.archived) throw new Error("Settings cannot be changed right now. Please retry.");
    inFlight.current = true; setPending(true);
    try {
      const result = await saveSubagentAssignments({ data: { agentId, id: definition.id, settings } });
      if (!result.ok) throw new Error(result.error);
      const { delegatable, persistent, timeoutSeconds } = result.data;
      setDefinition(result.data);
      setForm((current) => ({ ...current, delegatable, persistent, timeoutSeconds }));
      toast.success("Assignment settings updated");
    } finally { inFlight.current = false; setPending(false); }
  }
  async function saveIdentity(identity: Pick<SubagentInput, "name" | "description">) {
    if (!definition) { setForm((current) => ({ ...current, ...identity })); return; }
    if (inFlight.current || definition.archived) throw new Error("Settings cannot be changed right now. Please retry.");
    inFlight.current = true; setPending(true);
    try {
      const result = await saveSubagentIdentity({ data: { agentId, id: definition.id, ...identity } });
      if (!result.ok) throw new Error(result.error);
      setDefinition(result.data);
      setForm((current) => ({ ...current, name: result.data.name, description: result.data.description }));
      toast.success("Name and description updated");
    } finally { inFlight.current = false; setPending(false); }
  }
  async function saveTools(tools: string[]) {
    if (!definition) { set("tools", tools); return; }
    if (inFlight.current || definition.archived) throw new Error("Settings cannot be changed right now. Please retry.");
    inFlight.current = true; setPending(true);
    try {
      const result = await saveSubagentTools({ data: { agentId, id: definition.id, tools } });
      if (!result.ok) throw new Error(result.error);
      setDefinition(result.data);
      setForm((current) => ({ ...current, tools: result.data.tools }));
      toast.success("Tools updated");
    } finally { inFlight.current = false; setPending(false); }
  }
  async function refresh() {
    if (!definition) return;
    const response = await getSubagent({ data: { agentId, id: definition.id } });
    if (!response.ok) throw new Error(response.error);
    setRuns(response.data.runs); setSchedulerError(response.data.schedulerError);
  }
  async function startRun() {
    if (inFlight.current || !definition || !definition.enabled || definition.archived) throw new Error("This worker is unavailable for new assignments.");
    inFlight.current = true; setPending(true); setError("");
    try {
      if (assignmentId.current?.task !== task) assignmentId.current = { task, id: crypto.randomUUID() };
      let result;
      try { result = await runSubagent({ data: { agentId, id: definition.id, task, requestId: assignmentId.current!.id } }); }
      catch { throw new Error("Unable to confirm the assignment. Retry with the same task to reuse its request ID."); }
      if (!result.ok) throw new Error(result.error);
      assignmentId.current = null; setRunOpen(false); setTask("");
      toast.success("Assignment started", { description: "Refresh run history for its outcome." });
      try { await refresh(); }
      catch { setError("Assignment started, but run history could not be refreshed. Refresh to check its outcome."); }
    } finally { inFlight.current = false; setPending(false); }
  }
  async function perform(action: "create" | "archive" | "refresh" | "stop", runId = "") {
    if (inFlight.current) return;
    inFlight.current = true; setPending(true); setError("");
    try {
      if (action === "create") {
        creationId.current ??= crypto.randomUUID();
        const result = await saveSubagent({ data: { agentId, id: definition?.id ?? "", requestId: creationId.current, settings: form } });
        if (!result.ok) { setError(result.error); return; }
        toast.success("Sub-agent created");
        setDefinition(result.data); setForm(result.data);
        return;
      }
      if (action === "refresh") { await refresh(); return; }
      if (action === "stop") {
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
    } catch { setError("Unable to complete the operation. Reload to check current state."); }
    finally { inFlight.current = false; setPending(false); }
  }
  return <div className="w-full space-y-3">
    <form onSubmit={(event) => { event.preventDefault(); if (!definition) void perform("create"); }} className={panel}>
      <fieldset disabled={pending || definition?.archived} className="space-y-4 disabled:opacity-60">
        <SubagentIdentity value={form} main={main} agentId={agentId} enabled={form.enabled} onToggleEnabled={() => { void toggleEnabled(); }} onChange={saveIdentity} actions={definition && !main ? <>
          <button type="button" disabled={pending || !definition.enabled || definition.archived} aria-haspopup="dialog" onClick={() => setRunOpen(true)} className="inline-flex cursor-pointer items-center gap-2 rounded-full border border-violet-500 px-4 py-2 text-sm text-violet-300 hover:bg-violet-950 disabled:cursor-not-allowed disabled:opacity-50"><svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" className="size-4"><path d="m13 2-9 12h7l-1 8 10-12h-7z" /></svg>Run now</button>
          <button type="button" disabled={pending || definition.archived} aria-label="Archive worker" title="Archive worker" aria-haspopup="dialog" onClick={() => setArchiveOpen(true)} className="inline-flex cursor-pointer items-center justify-center rounded-full p-2 text-red-300 hover:bg-red-950 hover:text-red-200 disabled:cursor-not-allowed disabled:opacity-50">
            <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" className="size-5"><path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7" /></svg>
          </button>
        </> : undefined} />
        {main && <p className="text-sm text-zinc-400">This is the existing main chat. It cannot be disabled, archived, or deleted.</p>}
        {definition?.archived && <p className="text-sm text-amber-200">Archived — new assignments are disabled. History remains available.</p>}
        {definition && !selected && <p className="text-sm text-amber-200">Current model: {form.provider}/{form.modelId}. Reconnect its provider or select an available model.</p>}
        <ModelPicker choices={choices} value={selected ? modelKey(form) : ""} thinkingLevel={form.thinkingLevel} onChange={async (value, thinkingLevel) => { const choice = choices.find((entry) => modelKey(entry) === value); if (choice) await saveSection("model", { provider: choice.provider, modelId: choice.modelId, thinkingLevel }); }} disabled={pending} />
        <label className="block text-sm text-zinc-300">Instructions<textarea required rows={7} maxLength={32000} value={form.instructions} onChange={(event) => set("instructions", event.target.value)} className={inputClass} placeholder={main ? defaultMainInstructions : defaults.instructions} /></label>
        {instructionsChanged && <div className="flex justify-end gap-2">
          <button type="button" onClick={() => { set("instructions", definition.instructions); setError(""); }} className="cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-800">Reset</button>
          <button type="button" disabled={!form.instructions.trim()} onClick={() => { void saveInstructions(); }} className={button}>Save instructions</button>
        </div>}
        <ToolPicker tools={form.tools} main={main} onChange={saveTools} />
        {!main && <>
          <AssignmentConfig value={form} onChange={saveAssignments} />
          <ScheduleConfig value={form} onChange={(schedule) => saveSection("schedule", schedule)} />
        </>}
        {!definition && <button disabled={pending || !selected || !form.name.trim()} className={button}>{pending ? "Creating…" : "Create subagent"}</button>}
      </fieldset>
    </form>
    {runOpen && definition && !main && <RunSubagentDialog task={task} onTaskChange={setTask} onCancel={() => setRunOpen(false)} onRun={startRun} />}
    {definition && !main && <section className={panel}><header className="flex items-center justify-between"><h3 className="font-medium text-white">Run history · latest 50</h3><button disabled={pending} onClick={() => { void perform("refresh"); }} className={button}>Refresh</button></header>{schedulerError && <p role="alert" className="text-sm text-red-300">{schedulerError}</p>}{runs.length === 0 && <p className="text-sm text-zinc-500">No assignments yet.</p>}{runs.map((run) => <article key={run.id} className="space-y-2 rounded-xl border border-zinc-800 p-4"><p className="text-sm text-zinc-300">{run.status} · {run.source} · {new Date(run.createdAt).toISOString()}{(run.status === "queued" || run.status === "running") && <button disabled={pending} onClick={() => { void perform("stop", run.id); }} className="ml-4 cursor-pointer text-red-300 hover:underline">Stop assignment</button>}</p><p className="font-mono text-xs text-zinc-500">{run.id}</p><p className="whitespace-pre-wrap break-words text-sm text-zinc-300">{run.task}</p>{run.result && <details><summary className="cursor-pointer text-sm text-violet-300">Result</summary><pre className="mt-2 max-h-96 overflow-auto whitespace-pre-wrap break-words text-sm text-zinc-300">{run.result}</pre></details>}</article>)}</section>}
    {archiveOpen && definition && !main && <ArchiveSubagentDialog name={definition.name} onCancel={() => setArchiveOpen(false)} onConfirm={() => { setArchiveOpen(false); void perform("archive"); }} />}
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </div>;
}
