import { useRouter } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { toast } from "sonner";
import type { SubagentRow, SubagentRun } from "../db/subagent-schema.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { defaultMainInstructions, type AssignmentSettings, type SubagentInput, type SubagentSections } from "../agents/subagent-input.js";
import { archiveSubagent, runSubagent, saveSubagent, saveSubagentAssignments, saveSubagentIdentity, saveSubagentSection, saveSubagentTools, stopSubagentRun } from "../web/subagents.js";
import { subagentTemplates, templateDraft } from "../agents/subagent-templates.js";
import { WorkerWorkflowConfig } from "./worker-workflow-config.js";
import { ModelPicker, modelKey } from "./model-picker.js";
import { ProviderLogo } from "./provider-logo.js";
import type { RunPagination } from "../agents/subagent-history.js";
import { useSubagentRuns } from "./use-subagent-runs.js";
import { SubagentRunHistory } from "./subagent-run-history.js";
import { RunSubagentDialog } from "./run-subagent-dialog.js";
import { ArchiveSubagentDialog } from "./archive-subagent-dialog.js";
import { AssignmentConfig, ScheduleConfig, SubagentIdentity, ToolPicker } from "./subagent-config.js";

const panel = "space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6";
const inputClass = "mt-2 block w-full rounded-lg border border-zinc-700 bg-zinc-950 p-3 text-sm text-white";
const button = "cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50";
const defaults: SubagentInput = { name: "", description: "", instructions: "Complete the assigned task using available tools. Cite evidence and timestamps, distinguish facts from assumptions, and report missing or stale data. Do not invent access to unavailable tools.", provider: "", modelId: "", thinkingLevel: "", tools: [], enabled: true, delegatable: true, persistent: false, schedule: "", delaySeconds: 30, completion: "wake", timeoutSeconds: 900 };
export function SubagentDetailPage({ agentId, initial, choices, workers = [] }: { workers?: SubagentRow[]; agentId: string; initial?: { definition: SubagentRow; runs: SubagentRun[]; pagination?: RunPagination; schedulerError: string | null }; choices: ModelChoice[] }) {
  const router = useRouter();
  const [definition, setDefinition] = useState(initial?.definition);
  const [templateId, setTemplateId] = useState("");
  const [form, setForm] = useState<SubagentInput>(initial?.definition ?? { ...defaults, provider: choices[0]?.provider ?? "", modelId: choices[0]?.modelId ?? "" });
  const [task, setTask] = useState("");
  const [pending, setPending] = useState(false), [error, setError] = useState("");
  const [archiveOpen, setArchiveOpen] = useState(false);
  const [runOpen, setRunOpen] = useState(false);
  const inFlight = useRef(false), creationId = useRef<string | null>(null), assignmentId = useRef<{ task: string; id: string } | null>(null);
  const main = definition?.kind === "orchestrator";
  const { history: { runs, pagination, schedulerError }, loading, connectionError, setPage } = useSubagentRuns(agentId, main ? undefined : definition?.id, initial);
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
      const saved = section === "workflow" ? { workflow: row.workflow } : section === "enabled" ? { enabled: row.enabled } : section === "instructions" ? { instructions: row.instructions }
        : section === "model" ? { provider: row.provider, modelId: row.modelId, thinkingLevel: row.thinkingLevel }
        : { schedule: row.schedule, delaySeconds: row.delaySeconds, completion: row.completion };
      setDefinition(row); setForm((current) => ({ ...current, ...saved }));
      toast.success(section === "workflow" ? "Workflow settings updated" : section === "enabled" ? row.enabled ? "Sub-agent enabled" : "Sub-agent paused" : section === "instructions" ? "Instructions updated" : section === "model" ? "Model updated" : "Schedule updated");
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
      toast.success("Assignment started");
      await router.navigate({ to: "/agents/$agentId/sub-agents/$subagentId/runs/$runId", params: { agentId, subagentId: definition.id, runId: result.data.id } });
    } finally { inFlight.current = false; setPending(false); }
  }
  async function perform(action: "create" | "archive" | "stop", runId = "") {
    if (inFlight.current) return;
    inFlight.current = true; setPending(true); setError("");
    try {
      if (action === "create") {
        creationId.current ??= crypto.randomUUID();
        const result = await saveSubagent({ data: { agentId, id: definition?.id ?? "", requestId: creationId.current, templateId, settings: form } });
        if (!result.ok) { setError(result.error); return; }
        toast.success("Sub-agent created");
        setDefinition(result.data); setForm(result.data);
        return;
      }
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
    } catch { setError("Unable to complete the operation. Reload to check current state."); }
    finally { inFlight.current = false; setPending(false); }
  }
  return <div className="w-full space-y-3">
    {!definition && <section className={panel}><h2 className="font-medium text-white">Start from a bundled template</h2><p className="text-sm text-zinc-400">Editable local copies, never auto-upgraded. All templates create paused, fresh-context workers with automatic entries OFF. Select your own provider/model and configure the workflow before creating.</p><label className="block text-sm">Template<select aria-label="Worker template" className={`${inputClass} cursor-pointer`} value={templateId} onChange={(event) => {
      if ((form.name || form.instructions !== defaults.instructions || form.tools.length || form.workflow) && !window.confirm("Applying a template replaces this unsaved draft, including instructions, tools and workflow. Continue?")) return;
      const id = event.target.value; setTemplateId(id);
      setForm(id ? templateDraft(id, { provider: form.provider, modelId: form.modelId, thinkingLevel: form.thinkingLevel }) : { ...defaults, provider: form.provider, modelId: form.modelId, thinkingLevel: form.thinkingLevel });
    }}><option value="">Blank worker</option>{subagentTemplates.map((template) => <option key={template.id} value={template.id}>{template.name} · v{template.version}</option>)}</select></label>{templateId && <div className="space-y-2 text-sm text-zinc-300"><p>{subagentTemplates.find((template) => template.id === templateId)?.description}</p><p>Suggested schedule: {form.schedule} + {form.delaySeconds}s · completion {form.completion}</p><p>Requested tools: {form.tools.join(", ")}</p><details><summary className="cursor-pointer">Preview responsibilities</summary><pre className="mt-3 whitespace-pre-wrap text-xs">{form.instructions}</pre></details></div>}</section>}
    {definition?.templateOrigin && <p className="text-xs text-zinc-400">Origin: {definition.templateOrigin.id} v{definition.templateOrigin.version} · editable copy; saved settings, not the catalog, are authoritative.</p>}
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
        {definition && !selected && <p className="flex items-center gap-1.5 text-sm text-amber-200"><ProviderLogo provider={form.provider} className="h-4 w-4 shrink-0 rounded-sm border border-zinc-800 bg-white p-0.5" /><span>Current model: {form.provider}/{form.modelId}. Reconnect its provider or select an available model.</span></p>}
        <ModelPicker choices={choices} value={selected ? modelKey(form) : ""} thinkingLevel={form.thinkingLevel} onChange={async (value, thinkingLevel) => { const choice = choices.find((entry) => modelKey(entry) === value); if (choice) await saveSection("model", { provider: choice.provider, modelId: choice.modelId, thinkingLevel }); }} disabled={pending} />
        <label className="block text-sm text-zinc-300">Instructions<textarea required rows={7} maxLength={32000} value={form.instructions} onChange={(event) => set("instructions", event.target.value)} className={inputClass} placeholder={main ? defaultMainInstructions : defaults.instructions} /></label>
        {instructionsChanged && <div className="flex justify-end gap-2">
          <button type="button" onClick={() => { set("instructions", definition.instructions); setError(""); }} className="cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-800">Reset</button>
          <button type="button" disabled={!form.instructions.trim()} onClick={() => { void saveInstructions(); }} className={button}>Save instructions</button>
        </div>}
        <ToolPicker tools={form.tools} main={main} onChange={saveTools} />
        {form.tools.includes("submit_orders") && !form.tools.includes("record_trade_decision") && <p role="status" className="text-sm text-amber-200">Opening-order readiness: record_trade_decision is not selected. Existing permissions were not widened; select it explicitly for new exposure. Protection/reduction remains available.</p>}
        {form.workflow?.purpose === "trading" && ["get_trading_context", "record_trade_decision", "prepare_protected_entry", "get_protection_status", "submit_orders"].filter((tool) => !form.tools.includes(tool)).map((tool) => <p key={tool} role="status" className="text-xs text-amber-200">Trading workflow readiness: missing selected tool {tool}.</p>)}
        {!main && <>
          <WorkerWorkflowConfig key={JSON.stringify(form.workflow ?? null)} value={form.workflow} workers={workers} disabled={pending || definition?.archived} onSave={(workflow) => saveSection("workflow", { workflow })} />
          <AssignmentConfig value={form} onChange={saveAssignments} />
          <ScheduleConfig value={form} onChange={(schedule) => saveSection("schedule", schedule)} />
        </>}
        {!definition && <button disabled={pending || !selected || !form.name.trim()} className={button}>{pending ? "Creating…" : "Create subagent"}</button>}
      </fieldset>
    </form>
    {runOpen && definition && !main && <RunSubagentDialog task={task} onTaskChange={setTask} onCancel={() => setRunOpen(false)} onRun={startRun} />}
    {definition && !main && <section className={panel}><header className="flex items-center justify-between"><h3 className="font-medium text-white">Run history</h3></header>{schedulerError && <p role="alert" className="text-sm text-red-300">{schedulerError}</p>}{connectionError && <p role="status" className="text-sm text-amber-200">{connectionError}</p>}<SubagentRunHistory agentId={agentId} runs={runs} pending={pending} onStop={(id) => { void perform("stop", id); }} pagination={pagination} loading={loading} onPage={setPage} /></section>}
    {archiveOpen && definition && !main && <ArchiveSubagentDialog name={definition.name} onCancel={() => setArchiveOpen(false)} onConfirm={() => { setArchiveOpen(false); void perform("archive"); }} />}
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </div>;
}
