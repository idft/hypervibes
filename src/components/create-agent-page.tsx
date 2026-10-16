import { useRouter } from "@tanstack/react-router";
import { useRef, useState } from "react";
import type { AgentRow } from "../db/agent-schema.js";
import type { TradingAccountChoices } from "../hyperliquid/service.js";
import { formatMoneyText } from "../hyperliquid/decimal.js";
import type { ModelChoice } from "../providers/provider-service.js";
import { ModelPicker, modelKey } from "./model-picker.js";

export interface CreateAgentResult { ok: boolean; error?: string; data?: AgentRow }
export interface CreateSubaccountResult { ok: boolean; error?: string; data?: { address: string; created: boolean } }

export function CreateAgentPage({ choices, models, create, createSubaccount }: {
  choices: TradingAccountChoices;
  models: ModelChoice[];
  create: (name: string, requestId: string, accountSelection: string, model: ModelChoice) => Promise<CreateAgentResult>;
  createSubaccount: (name: string) => Promise<CreateSubaccountResult>;
}) {
  const router = useRouter();
  const [name, setName] = useState("");
  const [selection, setSelection] = useState<string>("main");
  const [selectedModel, setSelectedModel] = useState("");
  const model = models.find((choice) => modelKey(choice) === selectedModel);
  const [pending, setPending] = useState(false);
  const [subaccountPending, setSubaccountPending] = useState(false);
  const [error, setError] = useState("");
  const [subaccountStatus, setSubaccountStatus] = useState("");
  const request = useRef<{ settings: string; id: string } | null>(null);
  const inFlight = useRef(false);
  const subaccountInFlight = useRef(false);
  const trimmed = name.trim();
  const availableSelection = selection === "main" ? !choices.mainAssignedTo : choices.subaccounts.some((account) => account.address === selection && !account.assignedTo);
  const accountReady = choices.apiWalletReady && choices.accountModeSupported && !choices.lookupError && availableSelection;

  async function submit() {
    if (inFlight.current || !trimmed || !model || !accountReady) return;
    inFlight.current = true;
    setPending(true);
    setError("");
    try {
      const settings = JSON.stringify([trimmed, selection, selectedModel]);
      if (request.current?.settings !== settings) request.current = { settings, id: crypto.randomUUID() };
      const result = await create(trimmed, request.current.id, selection, model);
      if (!result.ok || !result.data) { setError(result.error ?? "Unable to create Agent"); return; }
      window.location.assign(`/agents/${result.data.id}`);
    } catch {
      setError("Unable to create Agent. Retry to safely check the same request.");
    } finally {
      inFlight.current = false;
      setPending(false);
    }
  }

  async function newSubaccount() {
    if (subaccountInFlight.current || !trimmed) return;
    subaccountInFlight.current = true;
    setSubaccountPending(true);
    setSubaccountStatus("");
    try {
      const result = await createSubaccount(trimmed);
      if (!result.ok || !result.data) throw new Error(result.error ?? "Sub-Account creation failed.");
      setSelection(result.data.address);
      setSubaccountStatus(result.data.created ? "Sub-account created." : "An existing matching Sub-account was found.");
      await router.invalidate();
    } catch (reason) {
      setSubaccountStatus(reason instanceof Error ? reason.message : "Sub-Account creation failed.");
    } finally {
      subaccountInFlight.current = false;
      setSubaccountPending(false);
    }
  }

  return (
    <div className="mx-auto max-w-3xl" data-agent-creation>
      <header className="border-b border-zinc-800 pb-6">
        <h1 className="text-3xl font-semibold tracking-tight text-white">Create New Agent</h1>
      </header>

      {!choices.apiWalletReady && (
        <p role="status" className="mt-6 rounded-2xl border border-amber-800/70 bg-amber-950/30 px-4 py-3 text-sm text-amber-200">
          Every agent requires a linked Hyperliquid account and an approved API key. <a href="/account" className="cursor-pointer underline">Set it up on the Account page</a> before creating an agent.
        </p>
      )}
      {choices.lookupError && (
        <p role="status" className="mt-6 rounded-2xl border border-red-900/60 bg-red-950/20 px-4 py-3 text-sm text-red-300">{choices.lookupError}</p>
      )}

      <form className="mt-6 space-y-6" onSubmit={(event) => { event.preventDefault(); void submit(); }}>
        <div>
          <label htmlFor="display_name" className="block text-sm font-medium text-zinc-300">Name</label>
          <input id="display_name" type="text" value={name} onChange={(event) => setName(event.target.value)} required maxLength={80} autoFocus placeholder="BTC Momentum"
            className="mt-2 block w-full rounded-xl border border-zinc-800 bg-zinc-950 px-3 py-2 text-sm text-white placeholder-zinc-600 focus:border-zinc-600 focus:outline-none" />
        </div>

        <ModelPicker choices={models} value={selectedModel} onChange={setSelectedModel} disabled={pending} />
        <a href="/providers" className="block cursor-pointer text-xs text-violet-300 hover:underline">Manage provider connections</a>

        <fieldset>
          <legend className="text-sm font-medium text-zinc-300">Trading account</legend>
          {choices.subaccountCapacity && <p className="mt-1 text-xs text-zinc-500">Sub-account capacity: {choices.subaccountCapacity}.</p>}
          <div className="mt-3 overflow-hidden rounded-xl border border-zinc-700 divide-y divide-zinc-800">
            <ChoiceRow
              label="Main account" address={choices.mainAddress} balance={choices.mainBalance}
              checked={selection === "main"} disabled={Boolean(choices.mainAssignedTo)}
              note={choices.mainAssignedTo ? `Already assigned to ${choices.mainAssignedTo}` : null}
              onSelect={() => setSelection("main")} />
            {choices.subaccounts.map((subaccount) => (
              <ChoiceRow
                key={subaccount.address}
                label={subaccount.name ? `Sub-account: ${subaccount.name}` : "Sub-account"}
                address={subaccount.address} balance={subaccount.balance}
                checked={selection === subaccount.address} disabled={Boolean(subaccount.assignedTo)}
                note={subaccount.assignedTo ? `In use by ${subaccount.assignedTo.agentName}` : null}
                onSelect={() => setSelection(subaccount.address)} />
            ))}
            <div className="p-4">
              <button type="button" disabled={subaccountPending || !trimmed || !choices.apiWalletReady || !choices.accountModeSupported} onClick={() => { void newSubaccount(); }}
                className="cursor-pointer rounded-full border border-violet-500 px-5 py-2 text-sm font-medium text-violet-200 hover:border-violet-400 hover:bg-violet-950/40 hover:text-white disabled:cursor-wait disabled:opacity-50">
                {subaccountPending ? "Creating…" : "Create Sub-Account"}
              </button>
              <p aria-live="polite" className="mt-3 text-sm text-zinc-400">{subaccountStatus || (trimmed ? "" : "Enter an agent name first.")}</p>
            </div>
          </div>
        </fieldset>

        {error && <p role="alert" className="rounded-2xl border border-red-900/60 bg-red-950/20 p-4 text-sm text-red-300">{error}</p>}

        <div className="flex items-center gap-3 border-t border-zinc-800 pt-6">
          <button type="submit" disabled={pending || !trimmed || !model || !accountReady} className="cursor-pointer rounded-full bg-violet-600 px-5 py-2 text-sm font-medium text-white hover:bg-violet-500 disabled:cursor-wait disabled:opacity-50">{pending ? "Creating…" : "Create agent"}</button>
          <a href="/agents" className="cursor-pointer rounded-full border border-zinc-700 px-5 py-2 text-sm font-medium text-zinc-500 hover:border-zinc-600 hover:text-zinc-300">Cancel</a>
        </div>
      </form>
    </div>
  );
}

function ChoiceRow({ label, address, balance, checked, disabled, note, onSelect }: {
  label: string;
  address: string | null;
  balance: string | null;
  checked: boolean;
  disabled: boolean;
  note: string | null;
  onSelect(): void;
}) {
  return (
    <label className={`flex cursor-pointer gap-3 p-4 transition hover:bg-zinc-900/60 ${disabled ? "opacity-50" : ""}`}>
      <input className="peer sr-only" type="radio" name="trading_account_selection" checked={checked} disabled={disabled} onChange={onSelect} />
      <span className="peer-checked:border-violet-500 peer-checked:bg-violet-500 peer-checked:after:opacity-100 mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border-2 border-zinc-600 bg-zinc-950 transition after:size-1.5 after:rounded-full after:bg-white after:opacity-0 after:content-[''] peer-focus-visible:ring-2 peer-focus-visible:ring-violet-400" aria-hidden="true" />
      <span className="min-w-0">
        <span className="text-sm text-white">{label}</span>
        {address && <span className="mt-1 block break-all font-mono text-xs text-zinc-500">{address}{balance ? ` · ${formatMoneyText(balance)} USDC` : ""}</span>}
        {note && <span className="mt-1 block text-xs text-amber-300">{note}</span>}
      </span>
    </label>
  );
}