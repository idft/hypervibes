import { useRouter } from "@tanstack/react-router";
import { useRef, useState } from "react";
import type { AccountView } from "../hyperliquid/service.js";
import { BUILDER_RECIPIENT } from "../hyperliquid/constants.js";
import { formatMoneyText, moneyParts } from "../hyperliquid/decimal.js";
import { Identicon } from "./identicon.js";
import { SetupChecklist } from "./setup-checklist.js";
import {
  actionBase,
  approveAgentTypes,
  approveBuilderFeeTypes,
  signWalletAction,
  walletProvider,
} from "../web/wallet-action.js";

export interface AccountActionResult { ok: boolean; error?: string }

export interface AccountActions {
  setupApiWallet(source: "generate" | "import", privateKey: string): Promise<{ ok: boolean; error?: string; data?: string }>;
  approveApiWallet(request: unknown): Promise<AccountActionResult>;
  approveBuilderFee(request: unknown, feeBps: number): Promise<AccountActionResult>;
  cancelBuilderFee(request: unknown): Promise<AccountActionResult>;
}

function ExternalLinkIcon() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth="1.5" stroke="currentColor" className="h-4 w-4" aria-hidden="true">
      <path strokeLinecap="round" strokeLinejoin="round" d="M13.5 6H5.25A2.25 2.25 0 0 0 3 8.25v10.5A2.25 2.25 0 0 0 5.25 21h10.5A2.25 2.25 0 0 0 18 18.75V10.5m-10.5 6L21 3m0 0h-5.25M21 3v5.25" />
    </svg>
  );
}

function expiryClass(expiresAt: number | null): string {
  if (!expiresAt) return "text-zinc-400";
  const days = (expiresAt - Date.now()) / 86_400_000;
  if (days >= 150) return "text-emerald-400";
  if (days < 30) return "text-amber-400";
  return "text-zinc-400";
}

function Balance({ balance }: { balance: string | null }) {
  if (!balance) return <span className="text-amber-300">Unavailable</span>;
  const { whole, decimals } = moneyParts(formatMoneyText(balance));
  const zero = /^0(\.0+)?$/.test(balance);
  return (
    <>
      <span className={zero ? "text-zinc-500" : "text-zinc-200"}>{whole}</span>
      {decimals && <span className="text-zinc-500">.{decimals}</span>}
      <span className="text-zinc-500"> USDC</span>
    </>
  );
}

export function AccountPage({ view, actions, setup }: { view: AccountView; actions: AccountActions; setup: { agentCount: number; providerConnected: boolean } }) {
  const router = useRouter();
  const [signerStatus, setSignerStatus] = useState("");
  const [signerPending, setSignerPending] = useState(false);
  const [importing, setImporting] = useState(false);
  const [importedKey, setImportedKey] = useState("");
  const [feeBps, setFeeBps] = useState(view.builderFee.feeBps);
  const [feeStatus, setFeeStatus] = useState("");
  const [feePending, setFeePending] = useState(false);
  const signerInFlight = useRef(false);

  async function generateAndApprove(source: "generate" | "import", force: boolean) {
    if (signerInFlight.current) return;
    signerInFlight.current = true;
    setSignerPending(true);
    setSignerStatus("");
    try {
      let address = view.apiWallet?.address ?? null;
      if (force || !address) {
        setSignerStatus(force ? "Re-generating trading signer…" : "Generating trading signer…");
        const result = await actions.setupApiWallet(source, source === "import" ? importedKey : "");
        if (!result.ok || !result.data) throw new Error(result.error ?? "Could not generate trading signer.");
        address = result.data;
      }
      setSignerStatus("Awaiting wallet signature…");
      const signed = await signWalletAction({ provider: walletProvider() }, "HyperliquidTransaction:ApproveAgent", approveAgentTypes,
        { type: "approveAgent", ...actionBase(), agentAddress: address, agentName: "HyperVibes" });
      setSignerStatus("Approving trading signer…");
      const approval = await actions.approveApiWallet(signed);
      if (!approval.ok) throw new Error(approval.error ?? "Hyperliquid did not approve the trading signer.");
      setSignerStatus("");
      await router.invalidate();
    } catch (reason) {
      setSignerStatus(reason instanceof Error ? reason.message : "Trading signer setup failed.");
    } finally {
      signerInFlight.current = false;
      setSignerPending(false);
    }
  }

  async function submitBuilderFee(revoke: boolean) {
    const clamped = Math.min(Math.max(Math.round(feeBps), 1), 10);
    const maxFeeRate = revoke ? "0.00%" : `${(clamped / 100).toFixed(2)}%`;
    setFeePending(true);
    setFeeStatus("Awaiting wallet signature…");
    try {
      const signed = await signWalletAction({ provider: walletProvider() }, "HyperliquidTransaction:ApproveBuilderFee", approveBuilderFeeTypes,
        { type: "approveBuilderFee", ...actionBase(), maxFeeRate, builder: BUILDER_RECIPIENT });
      const result = revoke
        ? await actions.cancelBuilderFee(signed)
        : await actions.approveBuilderFee(signed, clamped);
      if (!result.ok) throw new Error(result.error ?? "Hyperliquid did not update the fee approval.");
      setFeeStatus("");
      await router.invalidate();
    } catch (reason) {
      setFeeStatus(reason instanceof Error ? reason.message : "Approval failed.");
    } finally {
      setFeePending(false);
    }
  }

  const apiWallet = view.apiWallet;
  return (
    <div className="mx-auto max-w-3xl space-y-4">
      <header className="flex items-start justify-between gap-4">
        <h1 className="flex min-w-0 items-center gap-3 text-2xl font-semibold text-white sm:text-3xl">
          <Identicon address={view.walletAddress} className="h-8 w-8 shrink-0 rounded-full" />
          <span className="min-w-0 break-all font-mono text-lg sm:text-3xl">{view.walletAddress}</span>
          <a href={`https://arbiscan.io/address/${view.walletAddress}`} target="_blank" rel="noopener noreferrer" title="View on Arbiscan" aria-label="View on Arbiscan" className="inline-flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded text-zinc-400 hover:text-zinc-200">
            <ExternalLinkIcon />
          </a>
        </h1>
      </header>

      <SetupChecklist agentCount={setup.agentCount} state={{ providerConnected: setup.providerConnected, apiKeyApproved: view.apiWallet?.state === "approved", builderFeeApproved: view.builderFee.approved }} />
      {view.accountModeMessage && (
        <p role="status" className={`rounded-2xl border px-4 py-3 text-sm ${view.accountMode === "unregistered" ? "border-violet-900/70 bg-violet-950/20 text-zinc-300" : "border-amber-800/70 bg-amber-950/30 text-amber-200"}`}>{view.accountModeMessage}</p>
      )}
      {!view.encryptionConfigured && (
        <p role="status" className="rounded-2xl border border-amber-800/70 bg-amber-950/30 px-4 py-3 text-sm text-amber-200">Trading signer storage is not configured on the server. Set AGENTS_ENCRYPTION_KEY to enable API keys.</p>
      )}

      {view.accountMode === "unregistered" ? (
        <section className="rounded-2xl border border-violet-900/70 bg-violet-950/20 p-6 sm:p-8">
          <p className="text-sm font-medium uppercase tracking-[0.18em] text-violet-300">Hyperliquid setup</p>
          <h2 className="mt-3 text-2xl font-semibold text-white">Enable trading on Hyperliquid</h2>
          <p className="mt-3 max-w-2xl text-sm leading-6 text-zinc-300">Open Hyperliquid with this same wallet, then choose <span className="font-medium text-white">Enable Trading</span> and sign its gas-less transaction. Complete the first deposit Hyperliquid requests to activate the new account, then return here and refresh the page to set up your trading signer, builder-fee approval, and agents.</p>
          <a href="https://app.hyperliquid.xyz/trade" target="_blank" rel="noopener noreferrer" className="mt-6 inline-flex cursor-pointer items-center gap-2 rounded-full bg-violet-600 px-5 py-2.5 text-sm font-medium text-white hover:bg-violet-500">Open Hyperliquid<ExternalLinkIcon /></a>
        </section>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          <section className="flex flex-col rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
            <div className="flex items-start justify-between gap-3">
              <h2 className="flex items-center gap-2 text-lg font-medium text-white">Hyperliquid API Key
                <a href="https://app.hyperliquid.xyz/API" target="_blank" rel="noopener noreferrer" title="Open Hyperliquid API page" aria-label="Open Hyperliquid API page" className="inline-flex h-4 w-4 cursor-pointer items-center justify-center rounded text-zinc-400 hover:text-zinc-200">
                  <ExternalLinkIcon />
                </a>
              </h2>
              {apiWallet?.state === "approved"
                ? <span className="flex items-center gap-2">
                  {apiWallet.expiresAt && <span className={`shrink-0 text-sm ${expiryClass(apiWallet.expiresAt)}`}>Expires {new Date(apiWallet.expiresAt).toLocaleDateString()}</span>}
                  <span title="API key is approved" aria-label="API key is approved" className="inline-flex h-6 w-6 shrink-0 items-center justify-center text-emerald-400">
                    <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth="1.5" stroke="currentColor" className="h-5 w-5" aria-hidden="true"><path strokeLinecap="round" strokeLinejoin="round" d="m4.5 12.75 6 6 9-13.5" /></svg>
                  </span>
                </span>
                : <span className="inline-flex items-center gap-1.5 text-sm font-medium text-amber-400">
                  <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth="1.5" stroke="currentColor" className="h-5 w-5" aria-hidden="true"><path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z" /></svg>
                  Required
                </span>}
            </div>
            {apiWallet && (apiWallet.state === "approved" || apiWallet.showExpired) && apiWallet.address && (
              <div className="mt-3 space-y-1">
                <p className="break-all font-mono text-base text-zinc-200">{apiWallet.address}</p>
                {apiWallet.showExpired && <p className="text-sm font-semibold text-red-400">Expired</p>}
              </div>
            )}
            {apiWallet?.state === "approval-required" && apiWallet.address && (
              <p className="mt-3 break-all font-mono text-sm text-zinc-400">{apiWallet.address} — approval required</p>
            )}
            <p aria-live="polite" className="mt-3 text-sm text-zinc-400">{signerStatus}</p>
            <div className="mt-auto space-y-3 pt-4">
              {!apiWallet || apiWallet.state !== "approved" ? (
                <div className="flex items-center gap-4">
                  {importing && (
                    <input
                      type="password" autoComplete="off" placeholder="Private key" value={importedKey}
                      onChange={(event) => setImportedKey(event.target.value)}
                      className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-white" aria-label="Private key" />
                  )}
                  {apiWallet?.state === "approval-required"
                    ? <button type="button" disabled={signerPending} onClick={() => { void generateAndApprove("generate", false); }} className="cursor-pointer rounded-full bg-violet-600 px-5 py-2 text-sm font-medium text-white hover:bg-violet-500 disabled:cursor-wait disabled:opacity-50">{signerPending ? "Working…" : "Approve API Key"}</button>
                    : <button type="button" disabled={signerPending || (importing && !importedKey.trim())} onClick={() => { void generateAndApprove(importing ? "import" : "generate", false); }} className="cursor-pointer rounded-full bg-violet-600 px-5 py-2 text-sm font-medium text-white hover:bg-violet-500 disabled:cursor-wait disabled:opacity-50">{signerPending ? "Working…" : importing ? "Import API Key" : "Generate API Key"}</button>}
                  <button type="button" aria-expanded={importing} onClick={() => { setImporting(!importing); setImportedKey(""); }} className="cursor-pointer text-sm text-zinc-400 underline hover:text-zinc-200">{importing ? "Generate a new API key" : "Import existing API key"}</button>
                </div>
              ) : (
                <button type="button" disabled={signerPending} onClick={() => { void generateAndApprove("generate", true); }} className="cursor-pointer rounded-full border border-violet-500 px-5 py-2 text-sm font-medium text-violet-300 hover:border-violet-400 hover:bg-violet-950/30 disabled:cursor-wait disabled:opacity-50">{signerPending ? "Working…" : "Re-generate API key"}</button>
              )}
            </div>
          </section>

          <section className="flex flex-col rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
            <div className="flex items-start justify-between gap-3">
              <h2 className="flex items-center gap-2 text-lg font-medium text-white">Builder fee approval
                <a href="https://app.hyperliquid.xyz/builderCodes" target="_blank" rel="noopener noreferrer" title="Open Hyperliquid builder codes page" aria-label="Open Hyperliquid builder codes page" className="inline-flex h-4 w-4 cursor-pointer items-center justify-center rounded text-zinc-400 hover:text-zinc-200">
                  <ExternalLinkIcon />
                </a>
              </h2>
              {view.builderFee.approved
                ? <span title="Builder fee is approved" aria-label="Builder fee is approved" className="inline-flex h-6 w-6 shrink-0 items-center justify-center text-emerald-400">
                  <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth="1.5" stroke="currentColor" className="h-5 w-5" aria-hidden="true"><path strokeLinecap="round" strokeLinejoin="round" d="m4.5 12.75 6 6 9-13.5" /></svg>
                </span>
                : <span className="inline-flex items-center gap-1.5 text-sm font-medium text-amber-400">
                  <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" strokeWidth="1.5" stroke="currentColor" className="h-5 w-5" aria-hidden="true"><path strokeLinecap="round" strokeLinejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z" /></svg>
                  Required
                </span>}
            </div>
            <div className="mt-6 flex items-center gap-3">
              <label className="sr-only" htmlFor="builder-fee-input">Builder fee percent</label>
              <input id="builder-fee-input" type="number" min="0.01" max="0.1" step="0.01" value={(feeBps / 100).toFixed(2)}
                onChange={(event) => { const parsed = Number(event.target.value); if (Number.isFinite(parsed)) setFeeBps(Math.round(parsed * 100)); }}
                className="w-20 shrink-0 rounded-lg border border-zinc-700 bg-zinc-900 px-2 py-1 text-right text-sm font-medium text-zinc-300 [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none" />
              <span aria-hidden="true" className="w-6 shrink-0 text-left text-sm text-zinc-300">%</span>
              <label className="sr-only" htmlFor="builder-fee-range">Builder fee basis points</label>
              <input id="builder-fee-range" type="range" min="1" max="10" value={feeBps} onChange={(event) => setFeeBps(Number(event.target.value))} className="flex-1 cursor-pointer" />
            </div>
            <p aria-live="polite" className="mt-3 text-sm text-zinc-400">{feeStatus}</p>
            <div className="mt-auto flex items-center gap-3 pt-4">
              <button type="button" disabled={feePending} onClick={() => { void submitBuilderFee(false); }} className="cursor-pointer rounded-full bg-violet-600 px-5 py-2 text-sm font-medium text-white hover:bg-violet-500 disabled:cursor-wait disabled:opacity-50">{feePending ? "Working…" : view.builderFee.approved ? "Change approval" : "Approve"}</button>
              {view.builderFee.approved && (
                <button type="button" disabled={feePending} onClick={() => { void submitBuilderFee(true); }} className="cursor-pointer rounded-full border border-zinc-600 px-5 py-2 text-sm font-medium text-zinc-300 hover:border-zinc-500 hover:bg-zinc-800/50 disabled:cursor-wait disabled:opacity-50">Cancel Approval</button>
              )}
            </div>
          </section>
        </div>
      )}

      <section className="rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6">
        <h2 className="text-lg font-medium text-white">Accounts</h2>
        {view.lookupError && <p className="mt-2 text-sm text-amber-300">{view.lookupError}</p>}
        <div className="mt-4 overflow-x-auto sm:overflow-visible">
          <table className="w-full min-w-[42rem] text-left text-sm">
            <thead className="border-b border-zinc-800 text-zinc-500">
              <tr><th className="px-3 py-3 font-medium">Name</th><th className="px-3 py-3 font-medium">Address</th><th className="px-3 py-3 font-medium">Agent</th><th className="px-3 py-3 text-right font-medium">Balance</th></tr>
            </thead>
            <tbody className="divide-y divide-zinc-800">
              {view.accounts.map((account) => (
                <tr key={account.address}>
                  <td className="px-3 py-3 text-white">{account.name}</td>
                  <td className="break-all px-3 py-3 font-mono text-xs text-zinc-400">{account.address}</td>
                  <td className="px-3 py-3">{account.agentId
                    ? <a href={`/agents/${account.agentId}`} className="cursor-pointer text-violet-300 underline">{account.agentName}</a>
                    : <span className="text-zinc-500">&mdash;</span>}</td>
                  <td className="px-3 py-3 text-right"><Balance balance={account.balance} /></td>
                </tr>
              ))}
              {view.totalBalance !== null && (
                <tr className="font-bold">
                  <td></td><td></td>
                  <td className="px-3 py-3">Total</td>
                  <td className="px-3 py-3 text-right"><Balance balance={view.totalBalance} /></td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}