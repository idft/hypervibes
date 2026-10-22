import { useEffect, useId, useRef, useState } from "react";
import { toast } from "sonner";
import { useMutation, useQueryClient, useSuspenseQuery } from "@tanstack/react-query";
import type { AuthEvent, AuthType } from "@earendil-works/pi-ai";
import type { LoginView, ProviderSettings } from "../providers/provider-service.js";
import { invalidateProviderQueries, providerMutationOptions, providerSettingsQueryOptions, setupQueryOptions, type ProviderOperation } from "../web/provider-queries.js";
import { choiceContextTokens, costCurrency, modelKey } from "./model-picker.js";
import { ProviderLogo } from "./provider-logo.js";
import { SetupChecklist } from "./setup-checklist.js";

const button = "cursor-pointer rounded-lg border border-zinc-700 px-3 py-2 text-sm hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50";
const primaryButton = "cursor-pointer rounded-lg bg-violet-600 px-3 py-2 text-sm text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50";
const dangerButton = "cursor-pointer rounded-lg border border-red-800 px-3 py-2 text-sm text-red-300 hover:bg-red-950 hover:text-red-200 disabled:cursor-not-allowed disabled:opacity-50";
const input = "w-full rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-sm text-white";

type ProviderSummary = ProviderSettings["providers"][number];

/** Connect buttons sit inside the provider's own row, so a bare auth-type label
 * reads better than repeating the provider name ("Anthropic API key"). */
function connectLabel(method: { type: AuthType }): string {
  return method.type === "oauth" ? "OAuth" : "API key";
}

export function ProvidersPage() {
  const client = useQueryClient();
  const signInTab = useRef<{ tab: Window; loginId: string; eventCount: number } | undefined>(undefined);
  function closeSignInTab() {
    signInTab.current?.tab.close();
    signInTab.current = undefined;
  }
  const { data: settings, error: refreshError } = useSuspenseQuery(providerSettingsQueryOptions());
  const { data: setup } = useSuspenseQuery(setupQueryOptions());
  const { agentCount } = setup;
  const mutation = useMutation<LoginView | undefined, Error, ProviderOperation>({
    ...providerMutationOptions(client),
    onError: (_error, operation) => {
      if (operation.type === "answer") closeSignInTab();
    },
    onSuccess: (login: LoginView | undefined, operation) => {
      if (operation.type === "begin" && login) {
        if (login.state === "complete") toast.success("Provider connected.");
        else setLogin(login);
      } else if (operation.type === "cancel") setLogin(undefined);
      else if (operation.type === "answer") setValue("");
    },
  });
  const [login, setLogin] = useState<LoginView | undefined>(settings.activeLogin);
  const [disconnecting, setDisconnecting] = useState<ProviderSummary | undefined>();
  const [value, setValue] = useState("");
  const [search, setSearch] = useState("");
  const [expanded, setExpanded] = useState<string | undefined>();
  const [streamError, setStreamError] = useState("");
  const pending = mutation.isPending;
  const error = streamError || mutation.error?.message || refreshError?.message;
  const active = login?.state === "pending";

  useEffect(() => () => closeSignInTab(), []);
  useEffect(() => {
    const waiting = signInTab.current;
    if (!waiting) return;
    if (!login || login.id !== waiting.loginId || login.state !== "pending") {
      closeSignInTab();
      return;
    }
    const event = login.events.slice(waiting.eventCount).find((event) => event.type === "auth_url");
    if (event?.type !== "auth_url") return;
    try {
      const url = new URL(event.url);
      if (url.protocol !== "https:" && url.protocol !== "http:") { closeSignInTab(); return; }
      if (!waiting.tab.closed) waiting.tab.location.replace(url.href);
      // Once navigated, the provider tab belongs to the user; don't close it.
      signInTab.current = undefined;
    } catch { closeSignInTab(); }
  }, [login]);
  useEffect(() => { setValue(""); }, [login?.prompt?.id]);
  useEffect(() => {
    if (!login || login.state !== "pending") return;
    let disposed = false;
    const source = new EventSource(`/api/providers/logins/${encodeURIComponent(login.id)}/events`);
    source.addEventListener("provider-login", (event) => {
      if (disposed) return;
      const snapshot = JSON.parse((event as MessageEvent<string>).data) as LoginView;
      setStreamError("");
      if (snapshot.state === "pending") { setLogin(snapshot); return; }
      source.close();
      setLogin(snapshot.state === "failed" ? snapshot : undefined);
      if (snapshot.state === "complete") toast.success("Provider connected.");
      void invalidateProviderQueries(client);
    });
    source.onerror = () => {
      if (disposed) return;
      if (source.readyState === EventSource.CLOSED) {
        setLogin(undefined);
        setStreamError("Sign-in status unavailable. Reload the page to reconnect.");
      } else setStreamError("Sign-in status unavailable. Reconnecting…");
    };
    source.addEventListener("stream-error", () => {
      source.close();
      if (!disposed) { setLogin(undefined); setStreamError("Sign-in status unavailable. Reload the page."); }
    });
    source.addEventListener("unauthorized", () => {
      source.close();
      if (!disposed) { setLogin(undefined); setStreamError("Session ended. Please sign in again."); }
    });
    return () => { disposed = true; source.close(); };
  }, [client, login?.id, login?.state]);

  function begin(providerId: string, type: AuthType) {
    setStreamError("");
    mutation.mutate({ type: "begin", providerId, authType: type });
  }
  function submitAnswer(answer: string) {
    const prompt = login?.prompt;
    if (!login || !prompt) return;
    if (pending) return;
    if (answer === "browser" && prompt.input.type === "select" && prompt.input.options.some((option) => option.id === "browser")) {
      closeSignInTab();
      try {
        // Open synchronously during the click, before the asynchronous auth URL arrives.
        const tab = window.open("about:blank", "_blank");
        if (tab) {
          tab.opener = null;
          signInTab.current = { tab, loginId: login.id, eventCount: login.events.length };
        }
      } catch { closeSignInTab(); }
    }
    mutation.mutate({ type: "answer", id: login.id, promptId: prompt.id, value: answer });
  }
  function cancelLogin() {
    closeSignInTab();
    if (!login) return;
    if (login.state !== "pending") { setLogin(undefined); return; }
    mutation.mutate({ type: "cancel", id: login.id });
  }
  function disconnect(provider: ProviderSummary) {
    mutation.mutate({ type: "disconnect", providerId: provider.id });
  }
  const connected = settings.providers.filter((provider) => provider.connected);
  const renderProvider = (provider: ProviderSummary) => {
    const models = settings.models.filter((model) => model.provider === provider.id);
    const open = expanded === provider.id && models.length > 0;
    return <section key={provider.id}>
      <div className="flex flex-wrap items-center justify-between gap-4 p-4">
      <div className="flex min-w-0 items-center gap-3">
        <ProviderLogo provider={provider.id} className="h-8 w-8 shrink-0 rounded-lg border border-zinc-800 bg-white p-1" />
        <div><h3 className="font-medium text-white">{provider.name}</h3><p className="mt-1 text-xs text-zinc-400"><span className={provider.connected ? provider.configured ? "text-emerald-400" : "text-amber-300" : undefined}>{provider.connected ? provider.configured ? `Configured · ${provider.authType === "oauth" ? "OAuth" : "API key"}` : "Connection needs attention" : "Not connected"}</span> · {models.length > 0
        ? <button type="button" className="cursor-pointer underline decoration-zinc-600 hover:text-white" aria-expanded={open} onClick={() => setExpanded(open ? undefined : provider.id)}>{models.length} chat models</button>
        : `${models.length} chat models`}</p></div>
      </div>
    <div className="flex flex-wrap gap-2">
      {provider.methods.map((method) => <button key={method.type} className={button} disabled={pending || active || !settings.storageReady} onClick={() => begin(provider.id, method.type)}>{provider.connected ? "Re-connect" : connectLabel(method)}</button>)}
      {!provider.methods.length && <span className="text-xs text-zinc-500">Ambient-only authentication is not available in per-user setup.</span>}
      {provider.connected && <button className={dangerButton} disabled={pending || active} onClick={() => setDisconnecting(provider)}>Disconnect</button>}
      </div>
      </div>
      {open && <div className="max-h-60 overflow-y-auto border-t border-zinc-800 px-4 py-3">
        <table className="w-full text-xs">
          <caption className="sr-only">{provider.name} chat models</caption>
          <thead>
            <tr className="text-left text-zinc-500">
              <th scope="col" className="py-1 pr-4 font-medium">Model</th>
              <th scope="col" className="py-1 pr-4 font-medium">ID</th>
              <th scope="col" className="py-1 pr-4 font-medium">Context</th>
              <th scope="col" className="py-1 pr-4 font-medium">Reasoning</th>
              <th scope="col" className="py-1 pr-4 text-right font-medium">Input / 1M</th>
              <th scope="col" className="py-1 text-right font-medium">Output / 1M</th>
            </tr>
          </thead>
          <tbody>
            {models.map((model) => <tr key={modelKey(model)} className="border-t border-zinc-800/60">
              <td className="max-w-48 truncate py-1.5 pr-4 text-zinc-200">{model.name}</td>
              <td className="max-w-48 truncate py-1.5 pr-4 text-zinc-500">{model.modelId}</td>
              <td className="whitespace-nowrap py-1.5 pr-4 text-zinc-400">{choiceContextTokens(model)}</td>
              <td className="whitespace-nowrap py-1.5 pr-4 text-zinc-400">{model.reasoning ? "Yes" : "—"}</td>
              <td className="whitespace-nowrap py-1.5 pr-4 text-right font-mono text-zinc-400">{costCurrency(model.inputCost)}</td>
              <td className="whitespace-nowrap py-1.5 text-right font-mono text-zinc-400">{costCurrency(model.outputCost)}</td>
            </tr>)}
          </tbody>
        </table>
      </div>}
    </section>;
  };
  const loginOpen = login !== undefined && (login.state === "pending" || login.state === "failed");
  return (
    // The page fills the viewport below the navbar and never scrolls itself:
    // connected providers and the search box stay visible while the full
    // provider list scrolls inside its own section.
    <div className="mx-auto flex h-[calc(100dvh-6.5rem)] max-w-4xl flex-col gap-6 overflow-hidden">
      <header className="shrink-0"><h1 className="text-3xl font-semibold text-white">Providers</h1><p className="mt-2 text-sm text-zinc-400">Connect your AI accounts. Credentials are encrypted and private to your user.</p></header>
      {!settings.storageReady && <p role="status" className="shrink-0 text-sm text-amber-200">Provider setup requires AGENTS_ENCRYPTION_KEY on the server.</p>}
      <SetupChecklist state={{ ...setup, providerConnected: connected.length > 0 }} agentCount={agentCount} />
      {error && <p role="alert" className="shrink-0 text-sm text-red-300">{error}</p>}
      {connected.length > 0 && <section aria-labelledby="connected-providers-heading" className="shrink-0 space-y-3">
        <h2 id="connected-providers-heading" className="text-lg font-semibold text-white">Connected Providers</h2>
        <div className="divide-y divide-zinc-800 rounded-xl border border-zinc-800">{connected.map(renderProvider)}</div>
      </section>}
      <section aria-labelledby="all-providers-heading" className="flex min-h-0 flex-1 flex-col gap-3">
        <h2 id="all-providers-heading" className="shrink-0 text-lg font-semibold text-white">All Providers</h2>
        <div className="flex shrink-0 flex-wrap gap-3">
          <input aria-label="Search providers" placeholder="Search providers…" className={`${input} flex-1`} value={search} onChange={(event) => setSearch(event.target.value)} />
          <button className={button} disabled={pending || active} onClick={() => mutation.mutate({ type: "refresh" })}>Refresh models</button>
        </div>
        <div className="min-h-0 flex-1 divide-y divide-zinc-800 overflow-y-auto rounded-xl border border-zinc-800">
          {/* Connected providers live in their own section above; only the
              remaining, not-yet-connected providers scroll in this list. */}
          {settings.providers.filter((provider) => !provider.connected && `${provider.name} ${provider.id}`.toLowerCase().includes(search.toLowerCase())).map(renderProvider)}
        </div>
      </section>
      {loginOpen && login && <ProviderLoginDialog
        login={login}
        providerName={settings.providers.find((provider) => provider.id === login.providerId)?.name ?? login.providerId}
        value={value}
        onValueChange={setValue}
        pending={pending}
        onSubmit={submitAnswer}
        onCancel={cancelLogin}
      />}
      {disconnecting && <DisconnectProviderDialog
        provider={disconnecting}
        onCancel={() => setDisconnecting(undefined)}
        onConfirm={() => { const provider = disconnecting; setDisconnecting(undefined); disconnect(provider); }}
      />}
    </div>
  );
}

function ProviderLoginDialog({ login, providerName, value, onValueChange, pending, onSubmit, onCancel }: {
  login: LoginView;
  providerName: string;
  value: string;
  onValueChange: (value: string) => void;
  pending: boolean;
  onSubmit: (value: string) => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const formId = useId();
  useEffect(() => {
    const element = ref.current!;
    element.showModal();
    return () => element.close();
  }, []);
  const prompt = login.prompt;
  const loginMethods = prompt?.input.type === "select" && prompt.input.options.length === 2
    && prompt.input.options.some((option) => option.id === "browser")
    && prompt.input.options.some((option) => option.id === "device_code")
    ? [...prompt.input.options].sort((a) => a.id === "browser" ? -1 : 1)
    : undefined;
  return <dialog ref={ref} aria-labelledby={titleId} onCancel={(event) => { event.preventDefault(); if (!pending) onCancel(); }} className="m-auto max-h-[calc(100dvh-2rem)] w-[calc(100%_-_2rem)] max-w-md overflow-y-auto rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6 text-zinc-100 shadow-2xl backdrop:bg-black/70 backdrop:backdrop-blur-sm">
    <h3 id={titleId} className="text-lg font-semibold text-white">{providerName} sign-in</h3>
    {login.events.length > 0 && <div className="mt-4 space-y-3">{login.events.map((event, index) => <LoginEvent key={index} event={event} />)}</div>}
    {login.state === "failed" && <p role="alert" className="mt-4 text-sm text-red-300">{login.error ?? "Sign-in failed."}</p>}
    {login.state === "pending" && prompt && <form id={formId} key={prompt.id} className="mt-4 space-y-3" onSubmit={(event) => { event.preventDefault(); onSubmit(value); }}>
      {loginMethods ? <fieldset disabled={pending}>
        <legend className="text-sm">{prompt.input.message}</legend>
        <div className="mt-3 grid grid-cols-2 gap-3">
          {loginMethods.map((option) => <button key={option.id} type="button" aria-label={option.label} title={option.description ? `${option.label} — ${option.description}` : option.label} className={`${button} flex flex-col items-center justify-center gap-2 py-5 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-400`} onClick={() => onSubmit(option.id)}>
            <svg aria-hidden="true" className="h-7 w-7" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
              {option.id === "browser" ? <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M3 9h18M7 6.5h.01M10 6.5h.01" /></> : <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="m7 9 3 3-3 3m6 0h4" /></>}
            </svg>
            <span>{option.id === "browser" ? "Browser (default)" : "Device code"}</span>
          </button>)}
        </div>
      </fieldset> : <><label htmlFor="provider-answer" className="block text-sm">{prompt.input.message}</label>
      {prompt.input.type === "select" ? <select id="provider-answer" className={input} value={value} onChange={(event) => onValueChange(event.target.value)} required>
        <option value="">Choose…</option>
        {prompt.input.options.map((option) => <option key={option.id} value={option.id}>{option.label}{option.description ? ` — ${option.description}` : ""}</option>)}
      </select> : <input id="provider-answer" className={input} autoComplete="off" type={prompt.input.type === "secret" ? "password" : "text"} value={value} maxLength={16000} placeholder={"placeholder" in prompt.input ? prompt.input.placeholder : undefined} onChange={(event) => onValueChange(event.target.value)} required={prompt.input.type !== "text"} />}</>}
    </form>}
    {login.state === "pending" && !prompt && <p role="status" className="mt-4 text-sm text-zinc-400">Waiting for provider…</p>}
    {login.state === "failed"
      ? <div className="mt-6 flex items-center justify-end gap-3"><button type="button" className={button} onClick={onCancel}>Close</button></div>
      : <div className="mt-6 flex items-center justify-between gap-3">
        <button type="button" className={button} disabled={pending} onClick={onCancel}>Cancel sign-in</button>
        {prompt && !loginMethods && <button type="submit" form={formId} className={primaryButton} disabled={pending || (prompt.input.type !== "text" && !value.trim())}>Continue</button>}
      </div>}
  </dialog>;
}

function DisconnectProviderDialog({ provider, onCancel, onConfirm }: { provider: ProviderSummary; onCancel: () => void; onConfirm: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const element = ref.current!;
    element.showModal();
    return () => element.close();
  }, []);
  return <dialog ref={ref} aria-labelledby={titleId} aria-describedby={`${titleId}-description`} onCancel={(event) => { event.preventDefault(); onCancel(); }} className="m-auto w-[calc(100%_-_2rem)] max-w-md rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6 text-zinc-100 shadow-2xl backdrop:bg-black/70 backdrop:backdrop-blur-sm">
    <h3 id={titleId} className="text-lg font-semibold text-white">Disconnect {provider.name}?</h3>
    <p id={`${titleId}-description`} className="mt-2 text-sm leading-6 text-zinc-400">Agents using it will need another model or a reconnection.</p>
    <div className="mt-6 flex flex-col-reverse gap-3 sm:flex-row sm:justify-end">
      <button type="button" autoFocus onClick={onCancel} className="cursor-pointer rounded-full border border-zinc-800 px-4 py-2 text-sm font-medium text-zinc-300 hover:border-zinc-700 hover:text-white">Cancel</button>
      <button type="button" onClick={onConfirm} className="cursor-pointer rounded-full bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-500">Disconnect</button>
    </div>
  </dialog>;
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