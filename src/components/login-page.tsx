import { useRef, useState } from "react";
import type { EIP1193Provider } from "viem";
import { authClient } from "../web/auth-client.js";
import { loginError, walletLogin } from "../web/wallet-login.js";
import { getLoginDestination } from "../web/providers.js";

export function LoginPage() {
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const inFlight = useRef(false);

  async function login() {
    if (inFlight.current) return;
    inFlight.current = true;
    setPending(true);
    setError("");
    try {
      await walletLogin({
        provider: (window as Window & { ethereum?: EIP1193Provider }).ethereum,
        origin: window.location.origin,
        onStatus: setStatus,
        async nonce() {
          const result = await authClient.siwe.nonce();
          if (result.error || !result.data) throw new Error("Unable to start sign-in. Please try again.");
          return result.data.nonce;
        },
        async verify(message, signature) {
          const result = await authClient.siwe.verify({ message, signature });
          if (result.error) throw new Error("Sign-in could not be verified. Please try again.");
        },
      });
      window.location.assign(await getLoginDestination());
    } catch (reason) {
      setError(loginError(reason));
      setStatus("");
      inFlight.current = false;
      setPending(false);
    }
  }

  return (
    <main className="grid min-h-dvh place-items-center overflow-y-auto p-6">
      <section className="w-full max-w-md rounded-2xl border border-zinc-800 bg-[#0d0d10] p-8" aria-labelledby="login-heading">
        <h1 id="login-heading" className="flex items-center justify-center gap-3 text-2xl font-semibold tracking-wide">
          <img src="/static/android-chrome-512x512.png" alt="" className="h-14 w-14 rounded-md object-cover" />
          <span>HyperVibes</span>
        </h1>
        <button type="button" disabled={pending} onClick={() => { void login(); }} className="mt-6 w-full cursor-pointer rounded-full bg-white px-4 py-2 text-sm font-medium text-black disabled:cursor-wait disabled:opacity-60">
          {pending ? "Signing in…" : "Login"}
        </button>
        <p role="status" className="mt-4 text-sm text-zinc-400">{status}</p>
        <p role="alert" className="mt-2 text-sm text-red-300">{error}</p>
      </section>
    </main>
  );
}
