import { useRouter } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { toast } from "sonner";
import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { defaultMainInstructions, type AssignmentSettings, type SubagentInput, type SubagentSections } from "../agents/subagent-input.js";
import { archiveSubagent, runSubagent, saveSubagentAssignments, saveSubagentIdentity, saveSubagentSection, saveSubagentTools, stopSubagentRun, unarchiveSubagent } from "../web/subagents.js";
import { ModelPicker, modelKey } from "./model-picker.js";
import { ProviderLogo } from "./provider-logo.js";
import { WorkerTriggerConfig } from "./worker-trigger-config.js";
import type { InboxRequestSummary, RunPagination } from "../agents/subagent-history.js";
import { useSubagentRuns } from "./use-subagent-runs.js";
import { SubagentRunHistory } from "./subagent-run-history.js";
import { RunSubagentDialog } from "./run-subagent-dialog.js";
import { ArchiveSubagentDialog } from "./archive-subagent-dialog.js";
import { AssignmentConfig, ScheduleConfig, SubagentIdentity, ToolPicker } from "./subagent-config.js";

const panel = "space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6";
const inputClass = "mt-2 block w-full rounded-lg border border-zinc-700 bg-zinc-950 p-3 text-sm text-white";
const button = "cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50";
export function SubagentDetailPage({ agentId, initial, choices }: { agentId: string; initial: { definition: SubagentRow; runs: SubagentRun[]; pagination?: RunPagination; schedulerError: string | null; workers?: { id: string; name: string; archived?: boolean; tools?: string[] }[]; inbox?: InboxRequestSummary[] }; choices: ModelChoice[] }) {
  const router = useRouter();
  const [definition, setDefinition] = useState(initial.definition);
  const [form, setForm] = useState<SubagentInput>(initial.definition);
  const [task, setTask] = useState("");
  const [pending, setPending] = useState(false), [error, setError] = useState("");
  const [archiveOpen, setArchiveOpen] = useState(false);
  const [runOpen, setRunOpen] = useState(false);
  const inFlight = useRef(false), assignmentId = useRef<{ task: string; id: string } | null>(null);
  const main = definition.kind === "orchestrator";
  const { history: { runs, pagination, schedulerError, schedule, inbox }, loading, connectionError, setPage } = useSubagentRuns(agentId, main ? undefined : definition.id, initial);
  const nextRunAt = schedule && schedule.updatedAt >= definition.updatedAt ? schedule.nextRunAt : definition.nextRunAt;
  const selected = choices.find((choice) => modelKey(choice) === modelKey(form));
  const set = <K extends keyof SubagentInput>(key: K, value: SubagentInput[K]) => setForm((current) => ({ ...current, [key]: value }));
  const instructionsChanged = form.instructions !== definition.instructions;
  async function saveSection<K extends keyof SubagentSections>(section: K, settings: SubagentSections[K]) {
    if (inFlight.current || definition.archived) throw new Error("Settings cannot be changed right now. Please retry.");
    inFlight.current = true; setPending(true);
    try {
      const result = await saveSubagentSection({ data: { agentId, id: definition.id, section, settings } });
      if (!result.ok) throw new Error(result.error);
      const row = result.data;
      const saved = section === "enabled" ? { enabled: row.enabled } : section === "instructions" ? { instructions: row.instructions }
        : section === "model" ? { provider: row.provider, modelId: row.modelId, thinkingLevel: row.thinkingLevel }
        : section === "triggers" ? { triggers: row.triggers } : { schedule: row.schedule, delaySeconds: row.delaySeconds, completion: row.completion };
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
  async function startRun() {
    if (inFlight.current || definition.archived) throw new Error("This worker is unavailable for new assignments.");
    inFlight.current = true; setPending(true); setError("");
    try {
      if (assignmentId.current?.task !== task) assignmentId.current = { task, id: crypto.randomUUID() };
      let result;
      try { result = await runSubagent({ data: { agentId, id: definition.id, task, requestId: assignmentId.current!.id } }); }
      catch { throw new Error("Unable to confirm the assignment. Retry with the same task to reuse its request ID."); }
      if (!result.ok) throw new Error(result.error);
      assignmentId.current = null; setRunOpen(false); setTask("");
      toast.success("Assignment started");
      await router.navigate({ to: "/agents/$agentId/sub-agents/$subagentId/runs/$runId", params: { agentId, subagentId: definition.id, runId: result.data.id } });
    } finally { inFlight.current = false; setPending(false); }
  }
  async function perform(action: "archive" | "unarchive" | "stop", runId = "") {
    if (inFlight.current) return;
    inFlight.current = true; setPending(true); setError("");
    try {
      if (action === "stop") {
        const result = await stopSubagentRun({ data: { agentId, runId } });
        if (!result.ok) { setError(result.error); return; }
        toast.success("Assignment stopped", { description: "Existing exchange orders were not cancelled." });
      } else if (action === "unarchive") {
        const result = await unarchiveSubagent({ data: { agentId, id: definition.id } });
        if (!result.ok) { setError(result.error); return; }
        setDefinition(result.data); setForm(result.data);
        toast.success("Sub-agent unarchived", { description: "Restored paused. Enable it when ready." });
        await router.invalidate();
      } else {
        const result = await archiveSubagent({ data: { agentId, id: definition.id } });
        if (!result.ok) { setError(result.error); return; }
        setDefinition((current) => ({ ...current, archived: true, enabled: false })); setForm((current) => ({ ...current, enabled: false })); toast.success("Sub-agent archived", { description: "History is preserved." });
        await router.invalidate();
        await router.navigate({ to: "/agents/$agentId/sub-agents", params: { agentId } });
        return;
      }
    } catch { setError("Unable to complete the operation. Reload to check current state."); }
    finally { inFlight.current = false; setPending(false); }
  }
  return <div className="w-full space-y-3">
    {definition.archived && !main && <section className={`${panel} flex flex-wrap items-center justify-between gap-3`}>
      <p className="text-sm text-amber-200">Archived — history is preserved. Unarchive to edit or run this sub-agent; it will remain paused.</p>
      <button type="button" disabled={pending} onClick={() => { void perform("unarchive"); }} className={button}>Unarchive</button>
    </section>}
    <form onSubmit={(event) => { event.preventDefault(); }} className={panel}>
      <fieldset disabled={pending || definition.archived} className="space-y-4 disabled:opacity-60">
        <SubagentIdentity value={form} main={main} agentId={agentId} enabled={form.enabled} onToggleEnabled={() => { void toggleEnabled(); }} onChange={saveIdentity} actions={!main ? <>
          <button type="button" disabled={pending || definition.archived} aria-haspopup="dialog" onClick={() => setRunOpen(true)} className="inline-flex cursor-pointer items-center gap-2 rounded-full border border-violet-500 px-4 py-2 text-sm text-violet-300 hover:bg-violet-950 disabled:cursor-not-allowed disabled:opacity-50"><svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" className="size-4"><path d="m13 2-9 12h7l-1 8 10-12h-7z" /></svg>Run now</button>
          <button type="button" disabled={pending || definition.archived} aria-label="Archive worker" title="Archive worker" aria-haspopup="dialog" onClick={() => setArchiveOpen(true)} className="inline-flex cursor-pointer items-center justify-center rounded-full p-2 text-red-300 hover:bg-red-950 hover:text-red-200 disabled:cursor-not-allowed disabled:opacity-50">
            <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" className="size-5"><path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7" /></svg>
          </button>
        </> : undefined} />
        {definition.archived && <p className="text-sm text-amber-200">Archived — new assignments are disabled. History remains available.</p>}
        {!selected && <p className="flex items-center gap-1.5 text-sm text-amber-200"><ProviderLogo provider={form.provider} className="h-4 w-4 shrink-0 rounded-sm border border-zinc-800 bg-white p-0.5" /><span>Current model: {form.provider}/{form.modelId}. Reconnect its provider or select an available model.</span></p>}
        <ModelPicker choices={choices} value={selected ? modelKey(form) : ""} thinkingLevel={form.thinkingLevel} onChange={async (value, thinkingLevel) => { const choice = choices.find((entry) => modelKey(entry) === value); if (choice) await saveSection("model", { provider: choice.provider, modelId: choice.modelId, thinkingLevel }); }} disabled={pending} />
        <div className={main ? "space-y-4" : "grid gap-6 md:grid-cols-2"}>
          <div className="min-w-0 space-y-4">
            <ToolPicker tools={form.tools} onChange={saveTools} />
            {form.tools.includes("submit_orders") && !form.tools.includes("record_trade_decision") && <p role="status" className="text-sm text-amber-200">Trading readiness: record_trade_decision is not selected. Existing permissions were not widened; select it explicitly for attributable trade rationale.</p>}
          </div>
          {!main && <div className="min-w-0 space-y-6">
            <AssignmentConfig value={form} onChange={saveAssignments} />
            <WorkerTriggerConfig value={form.triggers} workers={(initial.workers ?? []).filter((worker) => worker.id !== definition.id)} onChange={(triggers) => saveSection("triggers", { triggers })} />
            <ScheduleConfig value={form} nextRunAt={definition.enabled && !definition.archived ? nextRunAt : null} onChange={(schedule) => saveSection("schedule", schedule)} />
          </div>}
        </div>
        <label className="block text-sm text-zinc-300">Instructions<textarea required rows={7} maxLength={32000} value={form.instructions} onChange={(event) => set("instructions", event.target.value)} className={inputClass} placeholder={main ? defaultMainInstructions : undefined} /></label>
        {instructionsChanged && <div className="flex justify-end gap-2">
          <button type="button" onClick={() => { set("instructions", definition.instructions); setError(""); }} className="cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-800">Reset</button>
          <button type="button" disabled={!form.instructions.trim()} onClick={() => { void saveInstructions(); }} className={button}>Save instructions</button>
        </div>}
      </fieldset>
    </form>
    {runOpen && !main && <RunSubagentDialog task={task} onTaskChange={setTask} onCancel={() => setRunOpen(false)} onRun={startRun} />}
    {!main && !!inbox?.length && <section className={panel}><h3 className="font-medium text-white">Worker inbox</h3><p className="text-xs text-zinc-400">Pending and recently discarded requests (up to 25). Paused workers retain requests until resumed or discarded; expired requests will not execute.</p>{inbox.map((request) => <div key={request.id} className="space-y-1 border-t border-zinc-800 pt-3 text-sm"><p className="text-zinc-300">{request.intent} · {request.reason}</p><p className="break-words text-xs text-zinc-400">{request.preview}</p>{request.discardedReason ? <p className="text-xs text-amber-200">{request.discardedReason}</p> : <button type="button" disabled={pending} onClick={() => { void perform("stop", request.id); }} className="cursor-pointer text-xs text-red-300 hover:underline disabled:cursor-wait">Discard request</button>}</div>)}</section>}
    {!main && <section className={panel}><header className="flex items-center justify-between"><h3 className="font-medium text-white">Run history</h3></header>{schedulerError && <p role="alert" className="text-sm text-red-300">{schedulerError}</p>}{connectionError && <p role="status" className="text-sm text-amber-200">{connectionError}</p>}<SubagentRunHistory agentId={agentId} runs={runs} pending={pending} onStop={(id) => { void perform("stop", id); }} pagination={pagination} loading={loading} onPage={setPage} /></section>}
    {archiveOpen && !main && <ArchiveSubagentDialog name={definition.name} onCancel={() => setArchiveOpen(false)} onConfirm={() => { setArchiveOpen(false); void perform("archive"); }} />}
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </div>;
}
