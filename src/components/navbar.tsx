import { Link } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { authClient } from "../web/auth-client.js";

export function Navbar({ walletAddress }: { walletAddress: string }) {
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
    <div className="relative flex h-full items-center px-4">
      <Link to="/agents" className="cursor-pointer text-sm text-zinc-400 hover:text-white">Agents</Link>
      <Link to="/agents" className="absolute left-1/2 flex h-full -translate-x-1/2 cursor-pointer items-center gap-2 px-4 text-sm font-semibold tracking-wide text-zinc-100 hover:bg-zinc-800">
        <img src="/static/android-chrome-512x512.png" alt="" className="h-9 w-9 rounded-md object-cover" />
        <span>HyperVibes</span>
      </Link>
      <div className="ml-auto flex items-center gap-3">
        <span title={walletAddress} className="hidden text-xs text-zinc-400 sm:inline">{walletAddress.slice(0, 6)}…{walletAddress.slice(-4)}</span>
        <button type="button" disabled={pending} onClick={() => { void logout(); }} className="cursor-pointer rounded-lg px-3 py-2 text-sm text-zinc-400 hover:bg-zinc-800 hover:text-white disabled:cursor-wait disabled:opacity-60">{pending ? "Logging out…" : "Logout"}</button>
        {error && <span role="alert" className="text-xs text-red-300">{error}</span>}
        <a href="https://github.com/idft/hypervibes" target="_blank" rel="noopener noreferrer" aria-label="HyperVibes on GitHub" className="inline-flex h-9 w-9 cursor-pointer items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-white">
          <span className="github-icon h-5 w-5" aria-hidden="true" />
        </a>
      </div>
    </div>
  );
}
