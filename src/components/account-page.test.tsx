// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider, notifyManager, useSuspenseQuery } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AccountView } from "../hyperliquid/service.js";
const mocks = vi.hoisted(() => ({ account: vi.fn(), setup: vi.fn(), approve: vi.fn(), fee: vi.fn(), cancel: vi.fn(), sign: vi.fn() }));
vi.mock("../web/account.js", () => ({ getAccount: mocks.account }));
vi.mock("../web/wallet-action.js", () => ({
  actionBase: () => ({ nonce: 1 }), approveAgentTypes: {}, approveBuilderFeeTypes: {},
  signWalletAction: mocks.sign, walletProvider: () => ({}),
}));
import { AccountPage } from "./account-page.js";
import { accountQueryOptions } from "../web/account-queries.js";
notifyManager.setScheduler((callback) => { queueMicrotask(callback); return () => {}; });
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const initial: AccountView = {
  walletAddress: `0x${"a".repeat(40)}`, accountMode: "unified", accountModeMessage: null, encryptionConfigured: true,
  apiWallet: null, builderFee: { approved: true, feeBps: 3 }, accounts: [], totalBalance: "0", lookupError: null,
};
function Page() {
  const { data: view } = useSuspenseQuery(accountQueryOptions());
  return <AccountPage view={view} setup={{ agentCount: 0, providerConnected: true }} actions={{
    setupApiWallet: mocks.setup, approveApiWallet: mocks.approve, approveBuilderFee: mocks.fee, cancelBuilderFee: mocks.cancel,
  }} />;
}
describe("account query mutations", () => {
  let root: Root, container: HTMLDivElement, client: QueryClient;
  beforeEach(async () => {
    vi.resetAllMocks();
    client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity, staleTime: Infinity } } });
    client.setQueryData(accountQueryOptions().queryKey, initial);
    client.setQueryData(["setup"], {}); client.setQueryData(["trading-accounts"], {});
    mocks.account.mockResolvedValue(initial); mocks.setup.mockResolvedValue("0xsigner");
    mocks.sign.mockResolvedValue({ action: {}, signature: "0xsigned" });
    mocks.approve.mockResolvedValue(undefined); mocks.fee.mockResolvedValue(undefined); mocks.cancel.mockResolvedValue(undefined);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<QueryClientProvider client={client}><Page /></QueryClientProvider>));
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); client.clear(); });
  const button = (text: string) => [...container.querySelectorAll("button")].find((entry) => entry.textContent === text)!;
  it("reuses the stored signer after a rejected wallet signature and refreshes onboarding", async () => {
    mocks.account.mockResolvedValue({ ...initial, apiWallet: { address: "0xsigner", state: "approval-required", expiresAt: null, showExpired: false } });
    mocks.sign.mockRejectedValueOnce(new Error("Wallet signature cancelled"));
    await act(async () => button("Generate API Key").click());
    expect(container.textContent).toContain("Wallet signature cancelled");
    expect(mocks.setup).toHaveBeenCalledOnce(); expect(mocks.approve).not.toHaveBeenCalled();
    expect(client.getQueryState(["setup"])?.isInvalidated).toBe(true);
    expect(client.getQueryState(["trading-accounts"])?.isInvalidated).toBe(true);
    await act(async () => button("Approve API Key").click());
    expect(mocks.setup).toHaveBeenCalledOnce();
    expect(mocks.approve).toHaveBeenCalledWith({ action: {}, signature: "0xsigned" });
    expect(container.textContent).not.toContain("Wallet signature cancelled");
  });
  it("serializes fee approvals and uses the same mutation path for revocation", async () => {
    let resolve!: (value: unknown) => void;
    mocks.sign.mockImplementationOnce(() => new Promise((done) => { resolve = done; }));
    await act(async () => { button("Change approval").click(); button("Change approval").click(); });
    expect(mocks.sign).toHaveBeenCalledOnce(); expect(button("Working…").disabled).toBe(true);
    await act(async () => resolve({ action: {}, signature: "0xsigned" }));
    expect(mocks.fee).toHaveBeenCalledExactlyOnceWith({ action: {}, signature: "0xsigned" }, 3);
    await act(async () => button("Cancel Approval").click());
    expect(mocks.cancel).toHaveBeenCalledExactlyOnceWith({ action: {}, signature: "0xsigned" });
    expect(mocks.sign.mock.calls[1]?.[3]).toMatchObject({ maxFeeRate: "0.00%" });
  });
});
