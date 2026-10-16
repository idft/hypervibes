import { useRef, useState } from "react";
import type { AgentChat } from "../agents/agent-service.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { getAgentChat, selectAgentModel } from "../web/agents.js";
import { AgentSectionPage } from "./agent-section-page.js";
import { ModelPicker, modelKey } from "./model-picker.js";

/** Settings uses request/response snapshots only, never a chat subscription. */
export function AgentSettingsPage({ initial, choices }: { initial: AgentChat; choices: ModelChoice[] }) {
  const [chat, setChat] = useState(initial);
  const [selection, setSelection] = useState(modelKey(initial.model));
  const [thinking, setThinking] = useState(initial.thinkingLevel ?? "");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const inFlight = useRef(false);
  const selected = choices.find((choice) => modelKey(choice) === selection);
  async function apply() {
    if (inFlight.current || !selected) return;
    inFlight.current = true; setPending(true); setError(""); setNotice("");
    try {
      const result = await selectAgentModel({ data: { id: chat.agent.id, provider: selected.provider, modelId: selected.modelId, thinkingLevel: thinking } });
      if (!result.ok) setError(result.error);
      else setNotice("Model updated. Conversation history is preserved.");
      const refreshed = await getAgentChat({ data: { id: chat.agent.id } });
      if (refreshed.ok) setChat(refreshed.data);
      else setError(refreshed.error);
    } catch { setError("Unable to update or refresh model settings. Reload to check the current selection."); }
    finally { inFlight.current = false; setPending(false); }
  }
  return <div className="w-full space-y-2">
    <section className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
      <h2 className="text-lg font-medium text-white">Settings</h2>
      <dl className="mt-4 grid gap-4 sm:grid-cols-2"><div><dt className="text-xs uppercase tracking-wide text-zinc-500">Trading account</dt><dd className="mt-2 break-all font-mono text-sm text-zinc-300">{chat.agent.tradingAccountAddress ?? "No account linked (chat only)"}</dd></div><div><dt className="text-xs uppercase tracking-wide text-zinc-500">Agent status</dt><dd className="mt-2 text-sm text-zinc-300">{chat.agent.state}</dd></div></dl>
    </section>
    <section className="space-y-4 rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
      <h3 className="text-lg font-medium text-white">Agent model</h3>
      <p className="break-words text-sm text-zinc-400">Current: {chat.model.provider}/{chat.model.modelId}{chat.thinkingLevel ? ` · ${chat.thinkingLevel}` : ""}</p>
      {!chat.configured && <p className="text-sm text-amber-200">Reconnect this Agent’s provider or select an available model.</p>}
      <a href="/providers" className="inline-block cursor-pointer text-sm text-violet-300 hover:underline">Manage providers</a>
      {choices.length ? <>
        <ModelPicker choices={choices} value={selected ? selection : ""} onChange={(value) => { setSelection(value); setThinking(""); }} disabled={pending} />
        {Boolean(selected?.thinkingLevels.length) && <label className="block text-sm text-zinc-300">Thinking level<select aria-label="Thinking level" value={thinking} onChange={(event) => setThinking(event.target.value)} disabled={pending} className="ml-3 cursor-pointer rounded-lg border border-zinc-700 bg-zinc-950 p-2"><option value="">Default</option>{selected?.thinkingLevels.map((level) => <option key={level} value={level}>{level}</option>)}</select></label>}
        <button type="button" onClick={() => { void apply(); }} disabled={pending || !selected || chat.agent.state !== "ready"} className="cursor-pointer rounded-full bg-violet-600 px-4 py-2 text-sm text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50">{pending ? "Saving…" : "Apply model"}</button>
      </> : <p className="text-sm text-zinc-400">No models available. Connect a provider first.</p>}
      <p className="text-xs text-zinc-500">Models can only change while the Agent is idle; the server checks this when saving. Conversation history is preserved.</p>
      {notice && <p role="status" className="text-sm text-emerald-300">{notice}</p>}{error && <p role="alert" className="text-sm text-red-300">{error}</p>}
    </section>
    <AgentSectionPage title="Execution backend" description="Execution environment configuration will be available here." />
    <AgentSectionPage title="Instruments" description="Analysis and trading instrument selection will be available here." />
    <AgentSectionPage title="Telegram" description="Connect Telegram to this Agent’s main chat in a future update." />
  </div>;
}
