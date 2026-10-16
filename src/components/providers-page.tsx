import { useEffect, useRef, useState } from "react";
import type { AuthEvent, AuthType } from "@earendil-works/pi-ai";
import type { LoginView, ProviderSettings } from "../providers/provider-service.js";
import { answerProviderLogin, beginProviderLogin, cancelProviderLogin, disconnectProvider, getProviderLogin, getProviderSettings, refreshProviderModels } from "../web/providers.js";

const button = "cursor-pointer rounded-lg border border-zinc-700 px-3 py-2 text-sm hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50";
const input = "w-full rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-sm text-white";

export function ProvidersPage({ initial }: { initial: ProviderSettings }) {
  const [settings, setSettings] = useState(initial);
  const [login, setLogin] = useState<LoginView | undefined>(initial.activeLogin);
  const [value, setValue] = useState("");
  const [search, setSearch] = useState("");
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const inFlight = useRef(false);
  const active = login?.state === "pending";

  useEffect(() => { setValue(""); }, [login?.prompt?.id]);
  useEffect(() => {
    if (!login || login.state !== "pending") return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const result = await getProviderLogin({ data: { id: login.id } });
        if (disposed) return;
        if (!result.ok) { setError(result.error); setLogin(undefined); return; }
        if (result.data.state === "pending") {
          setLogin(result.data);
          timer = setTimeout(() => { void poll(); }, 750);
        } else {
          const updated = await getProviderSettings();
          if (!disposed) { setSettings(updated); setLogin(result.data); setError(""); }
        }
      } catch {
        if (!disposed) {
          setError("Sign-in status unavailable. Retrying…");
          timer = setTimeout(() => { void poll(); }, 2000);
        }
      }
    };
    timer = setTimeout(() => { void poll(); }, 200);
    return () => { disposed = true; clearTimeout(timer); };
  }, [login?.id, login?.state]);

  async function action(operation: () => Promise<void>) {
    if (inFlight.current) return;
    inFlight.current = true; setPending(true); setError("");
    try { await operation(); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Provider operation failed. Try again."); }
    finally { inFlight.current = false; setPending(false); }
  }
  function begin(providerId: string, type: AuthType) {
    void action(async () => {
      const result = await beginProviderLogin({ data: { providerId, type } });
      if (!result.ok) throw new Error(result.error);
      setLogin(result.data);
      if (result.data.state === "complete") setSettings(await getProviderSettings());
    });
  }
  const prompt = login?.prompt;
  const connected = settings.providers.filter((provider) => provider.connected);
  const renderProvider = (provider: ProviderSettings["providers"][number]) => <section key={provider.id} className="flex flex-wrap items-center justify-between gap-4 p-4">
    <div><h3 className="font-medium text-white">{provider.name}</h3><p className="mt-1 text-xs text-zinc-400"><span className={provider.connected ? provider.configured ? "text-emerald-400" : "text-amber-300" : undefined}>{provider.connected ? provider.configured ? `Configured · ${provider.authType === "oauth" ? "OAuth" : "API key"}` : "Connection needs attention" : "Not connected"}</span> · {settings.models.filter((model) => model.provider === provider.id).length} chat models</p></div>
    <div className="flex flex-wrap gap-2">
      {provider.methods.map((method) => <button key={method.type} className={button} disabled={pending || active || !settings.storageReady} onClick={() => begin(provider.id, method.type)}>{provider.connected ? "Replace: " : ""}{method.label}</button>)}
      {!provider.methods.length && <span className="text-xs text-zinc-500">Ambient-only authentication is not available in per-user setup.</span>}
      {provider.connected && <button className={button} disabled={pending || active} onClick={() => {
        if (!window.confirm(`Disconnect ${provider.name}? Agents using it will need another model or a reconnection.`)) return;
        void action(async () => {
          const result = await disconnectProvider({ data: { providerId: provider.id } });
          if (!result.ok) throw new Error(result.error);
          setSettings(await getProviderSettings());
        });
      }}>Disconnect</button>}
    </div>
  </section>;
  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <header><h1 className="text-3xl font-semibold text-white">Providers</h1><p className="mt-2 text-sm text-zinc-400">Connect your AI accounts. Credentials are encrypted and private to your user.</p></header>
      {!settings.storageReady && <p role="status" className="text-sm text-amber-200">Provider setup requires AGENTS_ENCRYPTION_KEY on the server.</p>}
      {!settings.models.length && <p className="rounded-xl border border-amber-800 p-4 text-sm text-amber-200">Connect at least one provider with an available chat model before creating an agent. Account settings and existing conversations remain accessible.</p>}
      {login && <section aria-label="Provider sign-in" className="space-y-4 rounded-xl border border-violet-800 bg-zinc-900 p-5">
        <h2 className="font-semibold">{settings.providers.find((provider) => provider.id === login.providerId)?.name ?? login.providerId} sign-in</h2>
        {login.events.map((event, index) => <LoginEvent key={index} event={event} />)}
        {active && prompt && <form key={prompt.id} className="space-y-3" onSubmit={(event) => {
          event.preventDefault();
          void action(async () => {
            const result = await answerProviderLogin({ data: { id: login.id, promptId: prompt.id, value } });
            if (!result.ok) throw new Error(result.error);
            setValue(""); setLogin(result.data);
          });
        }}>
          <label htmlFor="provider-answer" className="block text-sm">{prompt.input.message}</label>
          {prompt.input.type === "select" ? <select id="provider-answer" className={input} value={value} onChange={(event) => setValue(event.target.value)} required>
            <option value="">Choose…</option>
            {prompt.input.options.map((option) => <option key={option.id} value={option.id}>{option.label}{option.description ? ` — ${option.description}` : ""}</option>)}
          </select> : <input id="provider-answer" className={input} autoComplete="off" type={prompt.input.type === "secret" ? "password" : "text"} value={value} maxLength={16000} placeholder={prompt.input.placeholder} onChange={(event) => setValue(event.target.value)} required={prompt.input.type !== "text"} />}
          <button className={button} disabled={pending || (prompt.input.type !== "text" && !value.trim())}>Continue</button>
        </form>}
        {active && !prompt && <p role="status" className="text-sm text-zinc-400">Waiting for provider…</p>}
        {active ? <button className={button} disabled={pending} onClick={() => { void action(async () => {
          const result = await cancelProviderLogin({ data: { id: login.id } });
          if (!result.ok) throw new Error(result.error);
          setLogin(undefined); setSettings(await getProviderSettings());
        }); }}>Cancel sign-in</button> : <p role="status" className="text-sm">{login.state === "complete" ? "Provider connected." : login.error ?? "Sign-in cancelled."}</p>}
      </section>}
      {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
      {connected.length > 0 && <section aria-labelledby="connected-providers-heading" className="space-y-3">
        <h2 id="connected-providers-heading" className="text-lg font-semibold text-white">Connected Providers</h2>
        <div className="divide-y divide-zinc-800 rounded-xl border border-zinc-800">{connected.map(renderProvider)}</div>
      </section>}
      <section aria-labelledby="all-providers-heading" className="space-y-3">
        <h2 id="all-providers-heading" className="text-lg font-semibold text-white">All Providers</h2>
        <div className="flex flex-wrap gap-3">
          <input aria-label="Search providers" placeholder="Search providers…" className={`${input} flex-1`} value={search} onChange={(event) => setSearch(event.target.value)} />
          <button className={button} disabled={pending || active} onClick={() => { void action(async () => {
            const result = await refreshProviderModels();
            setSettings(await getProviderSettings());
            if (!result.ok) throw new Error(result.error);
          }); }}>Refresh models</button>
        </div>
        <div className="divide-y divide-zinc-800 rounded-xl border border-zinc-800">
          {settings.providers.filter((provider) => `${provider.name} ${provider.id}`.toLowerCase().includes(search.toLowerCase())).map(renderProvider)}
        </div>
      </section>
    </div>
  );
}

function SafeLink({ url, children }: { url: string; children: React.ReactNode }) {
  try { if (!["http:", "https:"].includes(new URL(url).protocol)) return null; } catch { return null; }
  return <a className="cursor-pointer break-all text-violet-300 underline" href={url} target="_blank" rel="noopener noreferrer">{children}</a>;
}
function DeviceCode({ code, url }: { code: string; url: string }) {
  const [copyState, setCopyState] = useState<"idle" | "copying" | "copied" | "failed">("idle");
  async function copy() {
    if (copyState === "copying") return;
    setCopyState("copying");
    try { await navigator.clipboard.writeText(code); setCopyState("copied"); }
    catch { setCopyState("failed"); }
  }
  return <div className="space-y-2 text-sm">
    <SafeLink url={url}>Open device authorization</SafeLink>
    <div className="flex flex-wrap items-center gap-3">
      <p>Code: <strong className="select-all font-mono">{code}</strong></p>
      <button type="button" className={button} disabled={copyState === "copying"} onClick={() => { void copy(); }}>Copy to clipboard</button>
    </div>
    <p role="status" className={copyState === "failed" ? "text-amber-300" : "text-emerald-400"}>{copyState === "copied" ? "Copied!" : copyState === "failed" ? "Unable to copy. Select and copy the code above." : copyState === "copying" ? "Copying…" : ""}</p>
  </div>;
}
function LoginEvent({ event }: { event: AuthEvent }) {
  if (event.type === "auth_url") return <p className="text-sm"><SafeLink url={event.url}>Open provider sign-in</SafeLink>{event.instructions && <span className="mt-1 block text-zinc-400">{event.instructions}</span>}</p>;
  if (event.type === "device_code") return <DeviceCode key={event.userCode} code={event.userCode} url={event.verificationUri} />;
  return <div className="text-sm text-zinc-400"><p>{event.message}</p>{event.type === "info" && event.links?.map((link, index) => <SafeLink key={index} url={link.url}>{link.label ?? link.url}</SafeLink>)}</div>;
}
