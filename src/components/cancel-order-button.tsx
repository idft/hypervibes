import { useEffect, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { randomUuid } from "../web/random-uuid.js";
import { cancelAgentOrder } from "../web/agents.js";

export function CancelOrderButton({ agentId, symbol, oid }: { agentId: string; symbol: string; oid: string }) {
  const request = useRef<string | null>(null);
  const inFlight = useRef(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const timeout = window.setTimeout(() => setArmed(false), 8_000);
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); setArmed(false); trigger.current?.focus(); }
    };
    document.addEventListener("keydown", escape);
    return () => { window.clearTimeout(timeout); document.removeEventListener("keydown", escape); };
  }, [armed]);
  const client = useQueryClient();
  const mutation = useMutation({ mutationFn: async () => {
    request.current ??= randomUuid();
    return cancelAgentOrder({ data: { id: agentId, symbol, oid, requestId: request.current } });
  }, onSettled: async () => {
    await client.invalidateQueries({ queryKey: ["agent-dashboard", agentId] });
  } });
  async function cancel() {
    if (inFlight.current) return;
    if (!armed) { setArmed(true); return; }
    setArmed(false);
    inFlight.current = true;
    try { await mutation.mutateAsync(); }
    catch { /* Keep the original request ID after an error or a lost response. */ }
    finally { inFlight.current = false; }
  }
  const result = mutation.data;
  const complete = result && !result.uncertain && result.results.length > 0 && result.results.every((order) => order.orderStatus === "canceled");
  const label = mutation.isPending ? `Canceling ${symbol} order ${oid}…` : armed ? `Confirm cancel ${symbol} order ${oid}` : request.current ? `Check / retry ${symbol} order ${oid} cancellation` : `Cancel ${symbol} order ${oid}`;
  return <div className="w-36 whitespace-normal">
    <div role="group" aria-label={`Cancel ${symbol} order ${oid}`} className="inline-flex h-9 w-36">
      <button ref={trigger} type="button" title={label} aria-label={label} aria-busy={mutation.isPending} disabled={mutation.isPending || Boolean(complete)} onClick={() => { void cancel(); }} className={`inline-flex h-full cursor-pointer items-center justify-start gap-2 overflow-hidden rounded-full border bg-transparent pl-1.5 text-xs text-red-300 transition-[width,border-color] duration-150 hover:text-red-200 disabled:cursor-not-allowed disabled:opacity-50 ${armed ? "w-36 border-red-900/60 hover:border-red-500" : "w-9 border-transparent"}`}>
        <svg xmlns="http://www.w3.org/2000/svg" aria-hidden="true" fill="none" viewBox="0 0 24 24" strokeWidth="1.5" stroke="currentColor" className="size-6 shrink-0"><path strokeLinecap="round" strokeLinejoin="round" d="m9.75 9.75 4.5 4.5m0-4.5-4.5 4.5M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" /></svg>
        {armed && <span className="shrink-0 whitespace-nowrap">Confirm cancel</span>}
      </button>
    </div>
    {mutation.error && <p role="alert" className="mt-2 text-xs text-red-300">{mutation.error.message} Retry uses the same request ID.</p>}
    {result && <div role="status" className={`mt-2 text-xs ${complete ? "text-emerald-300" : "text-amber-200"}`}>
      <p>{complete ? "Order canceled." : "Cancellation incomplete or unconfirmed. Check the order before issuing another request."}</p>
      {!complete && <p className="break-all">Cancel request: {result.requestId}</p>}
      {result.results.filter((order) => order.error).map((order) => <p key={order.id}>{order.error}</p>)}
      {result.warnings.map((warning) => <p key={warning}>{warning}</p>)}
    </div>}
  </div>;
}
