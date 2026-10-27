import { randomUuid } from "../web/random-uuid.js";
import { useEffect, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { closeAgentPosition } from "../web/agents.js";

export function ClosePositionButton({ agentId, symbol }: { agentId: string; symbol: string }) {
  const request = useRef<string | null>(null);
  const inFlight = useRef(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const [confirmation, setConfirmation] = useState<{ agentId: string; symbol: string } | null>(null);
  const armed = confirmation !== null && confirmation.agentId === agentId && confirmation.symbol === symbol;
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
  const client = useQueryClient();
  const mutation = useMutation({ mutationFn: async () => {
    request.current ??= randomUuid();
    return closeAgentPosition({ data: { id: agentId, symbol, requestId: request.current } });
  }, onSettled: async () => {
    await client.invalidateQueries({ queryKey: ["agent-dashboard", agentId] });
  } });
  async function close() {
    if (inFlight.current) return;
    if (!armed) { setConfirmation({ agentId, symbol }); return; }
    setConfirmation(null);
    inFlight.current = true;
    try {
      const result = await mutation.mutateAsync();
      // A new explicit click may close a known remainder; uncertainty must reuse the original ID.
      if (!result.cancel.uncertain && !result.close.uncertain && result.remainingSize !== null) request.current = null;
    } catch { /* Retain the ID after a lost response; never blindly resubmit a new command. */ }
    finally { inFlight.current = false; }
  }
  const result = mutation.data;
  const label = mutation.isPending ? `Closing ${symbol}…` : armed ? `Confirm close ${symbol} at market` : request.current ? `Check / retry ${symbol} close` : `Close ${symbol} at market`;
  return <div className="w-36 whitespace-normal">
    <div role="group" aria-label={`Close ${symbol} position`} className="inline-flex h-9 w-36">
      <button ref={trigger} type="button" title={label} aria-label={label} aria-busy={mutation.isPending} disabled={mutation.isPending} onClick={() => { void close(); }} className={`inline-flex h-full cursor-pointer items-center justify-start gap-2 overflow-hidden rounded-full border bg-transparent pl-1.5 text-xs text-red-300 transition-[width,border-color] duration-150 hover:text-red-200 disabled:cursor-wait disabled:opacity-50 ${armed ? "w-36 border-red-900/60 hover:border-red-500" : "w-9 border-transparent"}`}>
        <svg xmlns="http://www.w3.org/2000/svg" aria-hidden="true" fill="none" viewBox="0 0 24 24" strokeWidth="1.5" stroke="currentColor" className="size-6 shrink-0"><path strokeLinecap="round" strokeLinejoin="round" d="m9.75 9.75 4.5 4.5m0-4.5-4.5 4.5M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" /></svg>
        {armed && <span className="shrink-0 whitespace-nowrap">Confirm close</span>}
      </button>
    </div>
    {mutation.error && <p role="alert" className="mt-2 text-xs text-red-300">{mutation.error.message} Retry uses the same request ID.</p>}
    {result && <div role="status" className={`mt-2 text-xs ${result.complete ? "text-emerald-300" : "text-amber-200"}`}>
      <p>{result.complete ? "Position closed; no market orders remain." : `Close incomplete or unconfirmed. Remaining size: ${result.remainingSize ?? "unknown"}; orders: ${result.remainingOrders ?? "unknown"}.`}</p>
      {!result.complete && <p className="break-all">Close request: {result.close.requestId}</p>}
      {[...result.cancel.results, ...result.close.results].filter((order) => order.error).map((order) => <p key={order.id}>{order.error}</p>)}
      {result.warnings.map((warning) => <p key={warning}>{warning}</p>)}
    </div>}
  </div>;
}
