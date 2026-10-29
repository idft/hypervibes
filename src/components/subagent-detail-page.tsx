import { randomUuid } from "../web/random-uuid.js";
import { useRouter } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { useMutation, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import { cacheSubagent, subagentQueryOptions } from "../web/subagent-queries.js";
import { subagentMutations } from "../web/subagent-mutations.js";
import { toast } from "sonner";
import type { ModelChoice } from "../providers/provider-service.js";
import { defaultMainInstructions, type AssignmentSettings, type SubagentInput, type SubagentSections } from "../agents/subagent-input.js";
import { archiveSubagent, saveSubagentAssignments, saveSubagentIdentity, saveSubagentTools, unarchiveSubagent } from "../web/subagents.js";
import { ModelPicker, modelKey } from "./model-picker.js";
import { ProviderLogo } from "./provider-logo.js";
import { WorkerTriggerConfig } from "./worker-trigger-config.js";
import { useSubagentRuns } from "./use-subagent-runs.js";
import { SubagentRunHistory } from "./subagent-run-history.js";
import { RunSubagentDialog } from "./run-subagent-dialog.js";
import { ArchiveSubagentDialog } from "./archive-subagent-dialog.js";
import { AssignmentConfig, ScheduleConfig, SubagentIdentity, ToolPicker } from "./subagent-config.js";

const panel = "space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6";
const inputClass = "mt-2 block w-full rounded-lg border border-zinc-700 bg-zinc-950 p-3 text-sm text-white";
const button = "cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50";
export function SubagentDetailPage({ agentId, subagentId, choices }: { agentId: string; subagentId: string; choices: ModelChoice[] }) {
  const router = useRouter();
  const client = useQueryClient();
  const mutations = subagentMutations(client, agentId);
  const { data: initial, error: refreshError } = useSuspenseQuery(subagentQueryOptions(agentId, subagentId));
  const definition = initial.definition;
  const [form, setForm] = useState<SubagentInput>(initial.definition);
  const [task, setTask] = useState("");
  const mutation = useMutation({
    mutationFn: (operation: () => Promise<void>) => operation(),
    onError: (error) => toast.error(error.message || "Unable to complete the operation. Please retry."),
  });
  const pending = mutation.isPending;
  const error = mutation.error?.message ?? refreshError?.message;
  const [archiveOpen, setArchiveOpen] = useState(false);
  const [runOpen, setRunOpen] = useState(false);
  const inFlight = useRef(false), assignmentId = useRef<{ task: string; id: string } | null>(null);
  const main = definition.kind === "orchestrator";
  const { history: { runs, pagination, schedulerError, schedule }, loading, connectionError, setPage } = useSubagentRuns(agentId, main ? undefined : definition.id, initial);
  const nextRunAt = schedule && schedule.updatedAt >= definition.updatedAt ? schedule.nextRunAt : definition.nextRunAt;
  const selected = choices.find((choice) => modelKey(choice) === modelKey(form));
  const set = <K extends keyof SubagentInput>(key: K, value: SubagentInput[K]) => setForm((current) => ({ ...current, [key]: value }));
  const instructionsChanged = form.instructions !== definition.instructions;
  async function operate(operation: () => Promise<void>) {
    // Retain the synchronous re-entry guard; Query owns pending/error state.
    if (inFlight.current) throw new Error("An operation is already in progress. Please retry.");
    inFlight.current = true;
    try { await mutation.mutateAsync(operation); }
    finally { inFlight.current = false; }
  }
  async function saveSection<K extends keyof SubagentSections>(section: K, settings: SubagentSections[K]) {
    if (definition.archived) throw new Error("Settings cannot be changed right now. Please retry.");
    await operate(async () => {
      const row = await mutations.saveSection({ id: definition.id, section, settings });
      const saved = section === "enabled" ? { enabled: row.enabled } : section === "instructions" ? { instructions: row.instructions }
        : section === "model" ? { provider: row.provider, modelId: row.modelId, thinkingLevel: row.thinkingLevel }
        : section === "triggers" ? { triggers: row.triggers } : { schedule: row.schedule, delaySeconds: row.delaySeconds, completion: row.completion };
      setForm((current) => ({ ...current, ...saved }));
      toast.success(section === "enabled" ? row.enabled ? "Sub-agent enabled" : "Sub-agent paused" : section === "instructions" ? "Instructions updated" : section === "model" ? "Model updated" : "Schedule updated");
    });
  }
  async function toggleEnabled() {
    try { await saveSection("enabled", { enabled: !form.enabled }); }
    catch { /* Query displays the error. */ }
  }
  async function saveInstructions() {
    try { await saveSection("instructions", { instructions: form.instructions }); }
    catch { /* Query displays the error and preserves the draft. */ }
  }
  async function saveAssignments(settings: AssignmentSettings) {
    if (definition.archived) throw new Error("Settings cannot be changed right now. Please retry.");
    await operate(async () => {
      const row = await saveSubagentAssignments({ data: { agentId, id: definition.id, settings } });
      const { delegatable, persistent, timeoutSeconds } = row;
      setForm((current) => ({ ...current, delegatable, persistent, timeoutSeconds }));
      await cacheSubagent(client, agentId, row);
      toast.success("Assignment settings updated");
    });
  }
  async function saveIdentity(identity: Pick<SubagentInput, "name" | "description">) {
    if (definition.archived) throw new Error("Settings cannot be changed right now. Please retry.");
    await operate(async () => {
      const row = await saveSubagentIdentity({ data: { agentId, id: definition.id, ...identity } });
      setForm((current) => ({ ...current, name: row.name, description: row.description }));
      await cacheSubagent(client, agentId, row);
      toast.success("Name and description updated");
    });
  }
  async function saveTools(tools: string[]) {
    if (definition.archived) throw new Error("Settings cannot be changed right now. Please retry.");
    await operate(async () => {
      const row = await saveSubagentTools({ data: { agentId, id: definition.id, tools } });
      setForm((current) => ({ ...current, tools: row.tools }));
      await cacheSubagent(client, agentId, row);
      toast.success("Tools updated");
    });
  }
  async function startRun() {
    if (definition.archived) throw new Error("This worker is unavailable for new assignments.");
    await operate(async () => {
      if (assignmentId.current?.task !== task) assignmentId.current = { task, id: randomUuid() };
      const run = await mutations.run({ id: definition.id, task, requestId: assignmentId.current.id });
      assignmentId.current = null; setRunOpen(false); setTask("");
      toast.success("Assignment started");
      await router.navigate({ to: "/agents/$agentId/sub-agents/$subagentId/runs/$runId", params: { agentId, subagentId: definition.id, runId: run.id } });
    });
  }
  async function perform(action: "archive" | "unarchive" | "stop", runId = "") {
    if (inFlight.current) return;
    try {
      await operate(async () => {
        if (action === "stop") {
          await mutations.stop(runId);
          toast.success("Assignment stopped", { description: "Existing exchange orders were not cancelled." });
        } else if (action === "unarchive") {
          const row = await unarchiveSubagent({ data: { agentId, id: definition.id } });
          setForm(row);
          await cacheSubagent(client, agentId, row);
          toast.success("Sub-agent unarchived", { description: "Restored paused. Enable it when ready." });
        } else {
          await archiveSubagent({ data: { agentId, id: definition.id } });
          await cacheSubagent(client, agentId, { ...definition, archived: true, enabled: false });
          toast.success("Sub-agent archived", { description: "History is preserved." });
          await router.navigate({ to: "/agents/$agentId/sub-agents", params: { agentId } });
        }
      });
    } catch { /* Query retains the operation error. */ }
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
            <ToolPicker tools={form.tools} onChange={saveTools} main={main} />
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
          <button type="button" onClick={() => { set("instructions", definition.instructions); mutation.reset(); }} className="cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-800">Reset</button>
          <button type="button" disabled={!form.instructions.trim()} onClick={() => { void saveInstructions(); }} className={button}>Save instructions</button>
        </div>}
      </fieldset>
      {error && <p role="alert" className="mt-4 text-sm text-red-300">{error}</p>}
    </form>
    {runOpen && !main && <RunSubagentDialog task={task} onTaskChange={setTask} onCancel={() => setRunOpen(false)} onRun={startRun} />}
    {!main && <section className={panel}><header className="flex items-center justify-between"><h3 className="font-medium text-white">Run history</h3></header>{schedulerError && <p role="alert" className="text-sm text-red-300">{schedulerError}</p>}{connectionError && <p role="status" className="text-sm text-amber-200">{connectionError}</p>}<SubagentRunHistory agentId={agentId} runs={runs} pending={pending} onStop={(id) => { void perform("stop", id); }} pagination={pagination} loading={loading} onPage={setPage} /></section>}
    {archiveOpen && !main && <ArchiveSubagentDialog name={definition.name} onCancel={() => setArchiveOpen(false)} onConfirm={() => { setArchiveOpen(false); void perform("archive"); }} />}
  </div>;
}
