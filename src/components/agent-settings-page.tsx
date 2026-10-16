import type { AgentChat } from "../agents/agent-service.js";
import { AgentSectionPage } from "./agent-section-page.js";

export function AgentSettingsPage({ initial }: { initial: AgentChat }) {
  return <div className="w-full space-y-2">
    <section className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
      <h2 className="text-lg font-medium text-white">Settings</h2>
      <dl className="mt-4 grid gap-4 sm:grid-cols-2"><div><dt className="text-xs uppercase tracking-wide text-zinc-500">Trading account</dt><dd className="mt-2 break-all font-mono text-sm text-zinc-300">{initial.agent.tradingAccountAddress ?? "No account linked (chat only)"}</dd></div><div><dt className="text-xs uppercase tracking-wide text-zinc-500">Agent status</dt><dd className="mt-2 text-sm text-zinc-300">{initial.agent.state}</dd></div></dl>
      <a href={`/agents/${initial.agent.id}/sub-agents/${encodeURIComponent(`main:${initial.agent.id}`)}`} className="mt-5 inline-block cursor-pointer text-sm text-violet-300 hover:underline">Edit orchestrator model, thinking, instructions & tools →</a>
    </section>
    <AgentSectionPage title="Execution backend" description="Execution environment configuration will be available here." />
    <AgentSectionPage title="Instruments" description="Analysis and trading instrument selection will be available here." />
    <AgentSectionPage title="Telegram" description="Connect Telegram to this Agent’s main chat in a future update." />
  </div>;
}
