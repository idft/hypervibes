import { Link, useRouter } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { toast } from "sonner";
import { saveSubagentSection } from "../web/subagents.js";
import { SubagentEnabledToggle } from "./subagent-enabled-toggle.js";
import type { SubagentRow } from "../db/subagent-schema.js";
import { ProviderLogo } from "./provider-logo.js";

function WorkerEnabledToggle({ agentId, definition }: { agentId: string; definition: SubagentRow }) {
  const router = useRouter();
  const [enabled, setEnabled] = useState(definition.enabled);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  async function toggle() {
    if (inFlight.current) return;
    inFlight.current = true; setPending(true); setError("");
    try {
      const result = await saveSubagentSection({ data: { agentId, id: definition.id, section: "enabled", settings: { enabled: !enabled } } });
      if (!result.ok) throw new Error(result.error);
      setEnabled(result.data.enabled);
      toast.success(result.data.enabled ? "Sub-agent enabled" : "Sub-agent paused");
      await router.invalidate();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Unable to change enabled status. Please retry.");
    } finally { inFlight.current = false; setPending(false); }
  }
  return <div className="shrink-0 text-right">
    <SubagentEnabledToggle enabled={enabled} disabled={pending} onClick={() => { void toggle(); }} />
    {error && <p role="alert" className="mt-2 max-w-64 text-sm text-red-300">{error}</p>}
  </div>;
}

export function SubagentsPage({ agentId, definitions }: { agentId: string; definitions: SubagentRow[] }) {
  const [showArchived, setShowArchived] = useState(false);
  const archivedCount = definitions.filter((row) => row.archived).length;
  const visible = definitions.filter((row) => showArchived || !row.archived);
  return <section className="w-full space-y-5 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <header className="flex flex-wrap items-center justify-between gap-4"><h2 className="text-lg font-medium text-white">Sub-agents</h2><div className="flex items-center gap-3">
      {archivedCount > 0 && <button type="button" aria-pressed={showArchived} onClick={() => setShowArchived((current) => !current)} className="cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-800">{showArchived ? "Hide archived" : `Show archived (${archivedCount})`}</button>}
      <Link to="/agents/$agentId/sub-agents/new" params={{ agentId }} className="cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500">Create subagent</Link>
    </div></header>
    <ul className="space-y-3">{visible.map((row) => <li key={`${row.id}:${row.updatedAt}`} className="flex items-start gap-3 rounded-xl border border-zinc-800 p-4 hover:border-violet-700"><Link to="/agents/$agentId/sub-agents/$subagentId" params={{ agentId, subagentId: row.id }} className="block min-w-0 flex-1 cursor-pointer">
      <div className="flex flex-wrap items-center gap-3"><span className="font-medium text-white">{row.name}</span><span className={row.archived ? "rounded-full border border-amber-500/60 bg-amber-500/15 px-3 py-1 text-sm font-semibold text-amber-200" : "text-xs text-zinc-400"}>{row.kind === "orchestrator" ? "Main conversation" : row.archived ? "Archived" : ""}</span></div>
      <p className="mt-2 text-sm text-zinc-400">{row.description}</p><p className="mt-2 flex items-center gap-1.5 text-xs text-zinc-500"><ProviderLogo provider={row.provider} className="h-4 w-4 rounded-sm border border-zinc-800 bg-white p-0.5" /><span className="break-all">{row.provider}/{row.modelId}</span>{row.thinkingLevel && <span> · {row.thinkingLevel}</span>}<span> · {row.kind === "orchestrator" ? "Persistent" : row.persistent ? "Persistent context" : "Fresh context per assignment"}{row.schedule ? ` · ${row.schedule} UTC candle close + ${row.delaySeconds}s` : ""}</span></p>
    </Link>{row.kind !== "orchestrator" && !row.archived && <WorkerEnabledToggle agentId={agentId} definition={row} />}</li>)}</ul>
  </section>;
}
