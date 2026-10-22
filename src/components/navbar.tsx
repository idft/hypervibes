import { Link } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { authClient } from "../web/auth-client.js";
import { AgentEnablementControl } from "./agent-enablement-control.js";
import { Identicon } from "./identicon.js";
import { AgentSwitcher, type SwitcherAgent } from "./agent-switcher.js";

export function Navbar({ walletAddress, agents = [], selectedAgentId }: { walletAddress: string; agents?: SwitcherAgent[]; selectedAgentId?: string }) {
  const selected = agents.find((agent) => agent.id === selectedAgentId);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  async function logout() {
    if (inFlight.current) return;
    inFlight.current = true;
    setPending(true);
    setError("");
    try {
      const result = await authClient.signOut();
      if (result.error) throw new Error("Unable to log out. Please try again.");
      window.location.assign("/login");
    } catch {
      setError("Unable to log out. Please try again.");
      inFlight.current = false;
      setPending(false);
    }
  }
  return (
    <div className="relative flex h-full items-center pr-4">
      <AgentSwitcher agents={agents} selectedAgentId={selectedAgentId} />
      {selected && !selected.tradingEnabled && <Link to="/agents/$agentId/settings" params={{ agentId: selected.id }} hash="risk-management" title="New exposure is disabled. Reduce-only orders and cancellation remain available. Open risk management settings." className="shrink-0 cursor-pointer whitespace-nowrap rounded-lg px-2 py-1 text-xs text-amber-300 hover:bg-amber-950/50 hover:text-amber-200 sm:text-sm">Trading disabled</Link>}
      <Link to="/agents" className="absolute left-1/2 hidden h-full -translate-x-1/2 cursor-pointer items-center gap-2 px-4 text-sm font-semibold tracking-wide text-zinc-100 hover:bg-zinc-800 lg:flex">
        <img src="/static/logo.png" alt="" className="h-9 w-auto rounded-md object-contain" />
        <span>HyperVibes</span>
      </Link>
      <div className="ml-auto flex items-center gap-2 sm:gap-3">
        {selected && <AgentEnablementControl key={selected.id} id={selected.id} name={selected.name} enabled={selected.enabled} />}
        <Link to="/providers" title="Provider connections" aria-label="Provider connections" className="inline-flex h-9 w-9 cursor-pointer items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-white">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth="1.5" stroke="currentColor" className="size-6" aria-hidden="true">
            <path strokeLinecap="round" strokeLinejoin="round" d="M5.25 14.25h13.5m-13.5 0a3 3 0 0 1-3-3m3 3a3 3 0 1 0 0 6h13.5a3 3 0 1 0 0-6m-16.5-3a3 3 0 0 1 3-3h13.5a3 3 0 0 1 3 3m-19.5 0a4.5 4.5 0 0 1 .9-2.7L5.737 5.1a3.375 3.375 0 0 1 2.7-1.35h7.126c1.062 0 2.062.5 2.7 1.35l2.587 3.45a4.5 4.5 0 0 1 .9 2.7m0 0a3 3 0 0 1-3 3m0 3h.008v.008h-.008v-.008Zm0-6h.008v.008h-.008v-.008Zm-3 6h.008v.008h-.008v-.008Zm0-6h.008v.008h-.008v-.008Z" />
          </svg>
        </Link>
        <Link to="/account" title={walletAddress} className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-1 hover:bg-zinc-800">
          <Identicon address={walletAddress} className="h-5 w-5 rounded-full" />
          <span className="hidden font-mono text-sm text-zinc-200 sm:inline">{walletAddress.slice(0, 6)}…{walletAddress.slice(-4)}</span>
        </Link>
        <button type="button" title="Logout" aria-label="Logout" disabled={pending} onClick={() => { void logout(); }} className="inline-flex h-9 w-9 cursor-pointer items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-white disabled:cursor-wait disabled:opacity-60">
          <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth={1.5} stroke="currentColor" className="size-6" aria-hidden="true">
            <path strokeLinecap="round" strokeLinejoin="round" d="M15.75 9V5.25A2.25 2.25 0 0 0 13.5 3h-6a2.25 2.25 0 0 0-2.25 2.25v13.5A2.25 2.25 0 0 0 7.5 21h6a2.25 2.25 0 0 0 2.25-2.25V15m3 0 3-3m0 0-3-3m3 3H9" />
          </svg>
        </button>
        {pending && error && <span role="alert" className="text-xs text-red-300">{error}</span>}
        <a href="https://github.com/idft/hypervibes" target="_blank" rel="noopener noreferrer" aria-label="HyperVibes on GitHub" className="hidden h-9 w-9 cursor-pointer items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-white sm:inline-flex">
          <span className="github-icon h-5 w-5" aria-hidden="true" />
        </a>
      </div>
    </div>
  );
}
