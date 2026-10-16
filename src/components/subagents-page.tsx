import { Link } from "@tanstack/react-router";
import { useState } from "react";
import type { SubagentRow } from "../db/subagent-schema.js";

export function SubagentsPage({ agentId, definitions }: { agentId: string; definitions: SubagentRow[] }) {
  const [showArchived, setShowArchived] = useState(false);
  const archivedCount = definitions.filter((row) => row.archived).length;
  const visible = definitions.filter((row) => showArchived || !row.archived);
  return <section className="w-full space-y-5 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
    <header className="flex flex-wrap items-center justify-between gap-4"><h2 className="text-lg font-medium text-white">Sub-agents</h2><div className="flex items-center gap-3">
      {archivedCount > 0 && <button type="button" aria-pressed={showArchived} onClick={() => setShowArchived((current) => !current)} className="cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-800">{showArchived ? "Hide archived" : `Show archived (${archivedCount})`}</button>}
      <Link to="/agents/$agentId/sub-agents/new" params={{ agentId }} className="cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500">Create subagent</Link>
    </div></header>
    <ul className="space-y-3">{visible.map((row) => <li key={row.id}><Link to="/agents/$agentId/sub-agents/$subagentId" params={{ agentId, subagentId: row.id }} className="block cursor-pointer rounded-xl border border-zinc-800 p-4 hover:border-violet-700">
      <div className="flex flex-wrap items-center gap-3"><span className="font-medium text-white">{row.name}</span><span className="text-xs text-zinc-400">{row.kind === "orchestrator" ? "Main conversation · Protected" : row.archived ? "Archived" : row.enabled ? "Enabled" : "Disabled"}</span></div>
      <p className="mt-2 text-sm text-zinc-400">{row.description}</p><p className="mt-2 break-all text-xs text-zinc-500">{row.provider}/{row.modelId}{row.thinkingLevel ? ` · ${row.thinkingLevel}` : ""} · {row.kind === "orchestrator" ? "Persistent" : row.persistent ? "Persistent context" : "Fresh context per assignment"}{row.schedule ? ` · ${row.schedule} UTC candle close + ${row.delaySeconds}s` : ""}</p>
    </Link></li>)}</ul>
  </section>;
}
