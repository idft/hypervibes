import { useEffect, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { setAgentEnabled } from "../web/agents.js";
import { agentQueryOptions, agentsQueryOptions } from "../web/agent-queries.js";

export function AgentEnablementControl({ id, name, enabled, variant = "navbar" }: { id: string; name: string; enabled: boolean; variant?: "navbar" | "settings" }) {
  const client = useQueryClient();
  const router = useRouter();
  const inFlight = useRef(false);
  const lastAction = useRef<boolean | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const [confirmation, setConfirmation] = useState<{ id: string; enabled: boolean } | null>(null);
  const armed = confirmation !== null && confirmation.id === id && confirmation.enabled === enabled;
  function cancel() { setConfirmation(null); trigger.current?.focus(); }
  useEffect(() => {
    if (!confirmation) return;
    if (!armed) { setConfirmation(null); return; }
    const timeout = window.setTimeout(() => setConfirmation(null), 8_000);
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); setConfirmation(null); trigger.current?.focus(); }
    };
    document.addEventListener("keydown", escape);
    return () => { window.clearTimeout(timeout); document.removeEventListener("keydown", escape); };
  }, [confirmation, armed]);
  const mutation = useMutation({
    mutationFn: setAgentEnabled,
    onSettled: async () => {
      // Refresh even on cleanup failure: disablement is persisted first.
      await Promise.all([
        client.invalidateQueries({ queryKey: agentQueryOptions(id).queryKey }),
        client.invalidateQueries({ queryKey: agentsQueryOptions().queryKey }),
        client.invalidateQueries({ queryKey: ["agent-chat", id] }),
        router.invalidate(),
      ]);
    },
  });
  async function toggle(nextEnabled = !enabled) {
    if (inFlight.current) return;
    if (!nextEnabled && !armed) { setConfirmation({ id, enabled }); return; }
    setConfirmation(null);
    inFlight.current = true;
    lastAction.current = nextEnabled;
    try { await mutation.mutateAsync({ data: { id, enabled: nextEnabled } }); }
    catch { /* Mutation retains the public error; retrying disable also retries cleanup. */ }
    finally { inFlight.current = false; }
  }
  const enabling = !armed && !enabled;
  const label = armed ? "Confirm stop" : enabled ? variant === "navbar" ? "Emergency stop" : "Disable agent" : "Enable agent";
  return <div className="relative flex flex-wrap items-center gap-2">
    {(!enabled || variant === "settings") && <span role="status" className={`whitespace-nowrap text-xs ${enabled ? "text-emerald-300" : "text-amber-300"}`}>{enabled ? "Agent enabled" : "Agent disabled"}</span>}
    <div role="group" aria-label={`Agent controls for ${name}`} className={`inline-flex h-9 overflow-hidden rounded-lg border ${enabling ? "border-emerald-900/70 hover:border-emerald-500" : "border-red-900/70 hover:border-red-500"}`}>
    <button ref={trigger} type="button" title={label} aria-label={label} disabled={mutation.isPending} onClick={() => { void toggle(armed ? false : !enabled); }} className={`inline-flex h-full cursor-pointer items-center justify-center gap-1.5 bg-transparent px-2 text-xs disabled:cursor-wait disabled:opacity-60 ${enabling ? "text-emerald-300 hover:text-emerald-200" : "text-red-300 hover:text-red-200"}`}>
      {enabling && <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" className="size-4"><path d="m8 5 11 7-11 7V5Z" /></svg>}
      {!armed && enabled && variant === "navbar" ? <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="size-5"><path d="M8 3h8l5 5v8l-5 5H8l-5-5V8z" /><path d="M9 9h6v6H9z" /></svg> : label}
    </button>
    {armed && <button type="button" onClick={cancel} className={`cursor-pointer border-l bg-transparent px-2 text-xs text-zinc-400 hover:text-white ${enabling ? "border-emerald-900/70" : "border-red-900/70"}`}>Cancel</button>}
    </div>
    {mutation.error && <span role="alert" className="max-w-64 text-xs text-red-300">{mutation.error.message}{lastAction.current === false && <> Disablement may already be saved. <button type="button" disabled={mutation.isPending} onClick={() => { void toggle(false); }} className="cursor-pointer underline disabled:cursor-wait">Retry stop</button></>}</span>}
    {mutation.data?.stopWarning && <span role="status" className="max-w-64 text-xs text-amber-200">{mutation.data.stopWarning}</span>}
  </div>;
}
