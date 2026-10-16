import { createModels } from "@earendil-works/pi-ai/models";
import { fauxProvider } from "@earendil-works/pi-ai/providers/faux";
import { privateKeyToAccount } from "viem/accounts";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvUsers, hvWalletAddresses } from "../db/auth-schema.js";
import { hvHyperliquidSettings } from "../db/exchange-schema.js";
import { eq } from "drizzle-orm";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { BUILDER_RECIPIENT } from "./constants.js";
import { HyperliquidError } from "./errors.js";
import type { AccountFeedEvents } from "./live-dashboard.js";

const encryptionKey = "3f".repeat(32);
const mainWallet = privateKeyToAccount(`0x${"5a".repeat(32)}` as `0x${string}`);
const signerPrivateKey = `0x${"6b".repeat(32)}`;
const signerAddress = privateKeyToAccount(signerPrivateKey as `0x${string}`).address.toLowerCase();
const domain = { name: "HyperliquidSignTransaction", version: "1", chainId: 42_161, verifyingContract: "0x0000000000000000000000000000000000000000" } as const;
const approveAgentTypes = {
  "HyperliquidTransaction:ApproveAgent": [
    { name: "hyperliquidChain", type: "string" },
    { name: "agentAddress", type: "address" },
    { name: "agentName", type: "string" },
    { name: "nonce", type: "uint64" },
  ],
} as const;
const approveBuilderFeeTypes = {
  "HyperliquidTransaction:ApproveBuilderFee": [
    { name: "hyperliquidChain", type: "string" },
    { name: "maxFeeRate", type: "string" },
    { name: "builder", type: "address" },
    { name: "nonce", type: "uint64" },
  ],
} as const;

/** Signs exactly like the browser wallet would for an ApproveAgent action. */
async function signAgentAction(agentAddress: string) {
  const action = { type: "approveAgent", hyperliquidChain: "Mainnet", signatureChainId: "0xa4b1", agentAddress, agentName: "HyperVibes", nonce: Date.now() };
  const signature = await mainWallet.signTypedData({
    domain, types: approveAgentTypes, primaryType: "HyperliquidTransaction:ApproveAgent",
    message: { hyperliquidChain: "Mainnet", agentAddress: agentAddress as `0x${string}`, agentName: "HyperVibes", nonce: BigInt(action.nonce) },
  });
  return { action, signature };
}

/** Signs exactly like the browser wallet would for an ApproveBuilderFee action. */
async function signBuilderFeeAction(feeBps: number) {
  const maxFeeRate = `${(feeBps / 100).toFixed(2)}%`;
  const action = { type: "approveBuilderFee", hyperliquidChain: "Mainnet", signatureChainId: "0xa4b1", maxFeeRate, builder: BUILDER_RECIPIENT, nonce: Date.now() };
  const signature = await mainWallet.signTypedData({
    domain, types: approveBuilderFeeTypes, primaryType: "HyperliquidTransaction:ApproveBuilderFee",
    message: { hyperliquidChain: "Mainnet", maxFeeRate, builder: BUILDER_RECIPIENT, nonce: BigInt(action.nonce) },
  });
  return { action, signature };
}

/** Routes Hyperliquid POSTs to canned responses, mirroring mainnet payloads. */
function hyperliquidFetch(routes: Record<string, (body: Record<string, unknown>) => unknown>): typeof fetch {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const body = JSON.parse(String(init?.body ?? "{}")) as Record<string, unknown>;
    const route = Object.entries(routes).find(([path]) => url.endsWith(path));
    if (!route) return new Response(JSON.stringify({ error: `unexpected ${url}` }), { status: 404 });
    return new Response(JSON.stringify(route[1](body)), { status: 200, headers: { "content-type": "application/json" } });
  }) as unknown as typeof fetch;
}

function infoRoutes(overrides: Record<string, (body: Record<string, unknown>) => unknown> = {}) {
  // Hyperliquid reports the approved maximum; relaying an approval raises it.
  let builderFeeTenths = 50;
  const info = (body: Record<string, unknown>) => {
    const type = body.type as string;
    if (overrides[type]) return overrides[type](body);
    if (type === "spotClearinghouseState") return { balances: [{ coin: "USDC", total: "12.5" }] };
    if (type === "subAccounts") {
      return [{
        name: "vt-BTC Momentum", master: mainWallet.address, subAccountUser: "0x9999999999999999999999999999999999999999",
        spotState: { balances: [{ coin: "USDC", total: "100.25" }] },
      }];
    }
    if (type === "userRole") return { role: "user" };
    if (type === "userAbstraction") return "unifiedAccount";
    if (type === "userRateLimit") return { cumVlm: "150000" };
    if (type === "maxBuilderFee") return builderFeeTenths;
    throw new Error(`unhandled info type ${type}`);
  };
  return hyperliquidFetch({
    "/info": info,
    "/exchange": (body) => {
      const action = body.action as Record<string, unknown> | undefined;
      if (action?.type === "createSubAccount") {
        return { status: "ok", response: { type: "createSubAccount", data: "0x9999999999999999999999999999999999999999" } };
      }
      if (action?.type === "approveBuilderFee") {
        builderFeeTenths = Number(String(action.maxFeeRate).replace("%", "")) * 1000;
      }
      return { status: "ok" };
    },
  });
}

describe("Hyperliquid account service", () => {
  let directory: string;
  let runtime: AppRuntime;
  const user = { id: "owner", walletAddress: mainWallet.address.toLowerCase() };
  let feedEvents: Map<string, AccountFeedEvents>;

  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-hyperliquid-"));
    feedEvents = new Map();
    const models = createModels();
    models.setProvider(fauxProvider().provider);
    runtime = await createRuntime(testConfig({
      APP_SQLITE_PATH: join(directory, "app.sqlite"),
      AGENTS_ENCRYPTION_KEY: encryptionKey,
    }), {
      models, model: { provider: "faux", modelId: "faux-1" },
      accountFeedFactory: (address, events) => { feedEvents.set(address, events); return { close: async () => {} }; },
      journalHistoryFetcher: async () => [],
    });
    await runtime.db.insert(hvUsers).values({ id: user.id, name: "Owner", email: "owner@example.test" }).run();
  });
  afterEach(async () => {
    vi.unstubAllGlobals();
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("authorizes dashboard reads before any exchange lookup and handles unlinked agents", async () => {
    const agent = await runtime.agents.create(user.id, "Chat only", randomUUID());
    const fetch = vi.fn();
    vi.stubGlobal("fetch", fetch);
    await expect(runtime.hyperliquid.agentDashboard("someone-else", agent.id)).rejects.toMatchObject({ status: 404 });
    await expect(runtime.hyperliquid.agentDashboard(user.id, randomUUID())).rejects.toMatchObject({ status: 404 });
    expect(await runtime.hyperliquid.agentDashboard(user.id, agent.id)).toMatchObject({ address: null, positions: null, orders: null });
    await expect(runtime.hyperliquid.agentAccountStatus("someone-else", agent.id)).rejects.toMatchObject({ status: 404 });
    await expect(runtime.hyperliquid.agentAccountStatus(user.id, randomUUID())).rejects.toMatchObject({ status: 404 });
    expect(await runtime.hyperliquid.agentAccountStatus(user.id, agent.id)).toMatchObject({ address: null, positions: null, orders: null });
    expect(fetch).not.toHaveBeenCalled();
  });

  it("shares the dashboard feed with the Agent account tool and isolates assigned accounts", async () => {
    const address = `0x${"ab".repeat(20)}`;
    const other = `0x${"cd".repeat(20)}`;
    const one = await runtime.agents.create(user.id, "First", randomUUID(), address);
    const two = await runtime.agents.create(user.id, "Second", randomUUID(), other);
    const fetcher = vi.fn(async (_input: unknown, init: RequestInit) => {
      const { type, user: account } = JSON.parse(String(init.body)) as { type: string; user: string };
      if (type === "clearinghouseState") return Response.json({ marginSummary: { accountValue: account === address ? "10.123" : "20" }, assetPositions: [] });
      if (type === "spotClearinghouseState") return Response.json({ balances: [{ coin: "USDC", total: account === address ? "150" : "250", hold: "5" }] });
      if (type === "userAbstraction") return Response.json("unifiedAccount");
      return Response.json([]);
    });
    vi.stubGlobal("fetch", fetcher);
    runtime.hyperliquid.start();
    await vi.waitFor(() => expect(feedEvents.size).toBe(2));
    for (const events of feedEvents.values()) events.connected();
    await vi.waitFor(async () => expect((await runtime.hyperliquid.agentDashboard(user.id, one.id)).live?.sections.orders.fresh).toBe(true));
    await vi.waitFor(async () => expect((await runtime.hyperliquid.agentDashboard(user.id, two.id)).live?.sections.orders.fresh).toBe(true));
    fetcher.mockClear();
    expect(await runtime.hyperliquid.agentAccountStatus(user.id, one.id)).toMatchObject({ address, perpEquity: "10.123", spotUsdc: "150", spotUsdcHold: "5", accountAbstraction: "unifiedAccount", sections: { perps: { source: "live", fresh: true }, spot: { source: "live", fresh: true } } });
    expect(await runtime.hyperliquid.agentAccountStatus(user.id, two.id)).toMatchObject({ address: other, perpEquity: "20", spotUsdc: "250" });
    expect(feedEvents.size).toBe(2);
    expect(fetcher).toHaveBeenCalledTimes(2);
    for (const [, init] of fetcher.mock.calls) expect(JSON.parse(String(init.body)).type).toBe("userAbstraction");
  });

  async function signingFixture() {
    await runtime.db.insert(hvWalletAddresses).values({ id: randomUUID(), userId: user.id, address: user.walletAddress, chainId: 42161, isPrimary: true, createdAt: new Date() }).run();
    await runtime.hyperliquid.storeApiWallet(user.id, "import", signerPrivateKey);
    await runtime.db.update(hvHyperliquidSettings).set({ apiWalletApprovedAt: Date.now(), builderFeeApprovedAt: Date.now(), builderFeeTenthsOfBp: 10 }).where(eq(hvHyperliquidSettings.userId, user.id)).run();
  }

  it("verifies live owner/signer authority and uses vaultAddress only for owned subaccounts", async () => {
    await signingFixture();
    const subaccount = `0x${"ab".repeat(20)}`;
    const sub = await runtime.agents.create(user.id, "Sub", randomUUID(), subaccount);
    const main = await runtime.agents.create(user.id, "Main", randomUUID(), user.walletAddress);
    vi.stubGlobal("fetch", infoRoutes({ userRole: (body) => body.user === signerAddress
      ? { role: "agent", data: { user: user.walletAddress } }
      : body.user === subaccount ? { role: "subAccount", data: { master: user.walletAddress } } : { role: "user" } }));
    expect(await runtime.hyperliquid.orderAuthority(user.id, sub.id, new AbortController().signal, true)).toMatchObject({ account: subaccount, vaultAddress: subaccount, privateKey: signerPrivateKey, builder: { b: BUILDER_RECIPIENT, f: 10 } });
    const mainAuthority = await runtime.hyperliquid.orderAuthority(user.id, main.id, new AbortController().signal, true);
    expect(mainAuthority.account).toBe(user.walletAddress);
    expect(mainAuthority).not.toHaveProperty("vaultAddress");
    await expect(runtime.hyperliquid.orderAuthority("another-user", main.id, new AbortController().signal, true)).rejects.toMatchObject({ status: 404 });
  });

  it("fails closed on foreign accounts/signers and revoked fees, without blocking fee-free cancellation", async () => {
    await signingFixture();
    const agent = await runtime.agents.create(user.id, "Main", randomUUID(), user.walletAddress);
    vi.stubGlobal("fetch", infoRoutes({ userRole: (body) => body.user === signerAddress ? { role: "agent", data: { user: `0x${"ff".repeat(20)}` } } : { role: "user" } }));
    await expect(runtime.hyperliquid.orderAuthority(user.id, agent.id, new AbortController().signal, true)).rejects.toThrow("signer approval");
    vi.stubGlobal("fetch", infoRoutes({ userRole: (body) => body.user === signerAddress ? { role: "agent", data: { user: user.walletAddress } } : { role: "user" }, maxBuilderFee: () => 0 }));
    await expect(runtime.hyperliquid.orderAuthority(user.id, agent.id, new AbortController().signal, true)).rejects.toThrow("Builder fee");
    expect((await runtime.hyperliquid.orderAuthority(user.id, agent.id, new AbortController().signal, false)).builder.f).toBe(0);
    const foreign = await runtime.agents.create(user.id, "Foreign", randomUUID(), `0x${"ff".repeat(20)}`);
    await expect(runtime.hyperliquid.orderAuthority(user.id, foreign.id, new AbortController().signal, false)).rejects.toThrow("ownership");
    await runtime.db.update(hvHyperliquidSettings).set({ apiWalletExpiresAt: Date.now() - 1 }).where(eq(hvHyperliquidSettings.userId, user.id)).run();
    await expect(runtime.hyperliquid.orderAuthority(user.id, agent.id, new AbortController().signal, false)).rejects.toThrow("approve a trading signer");
  });

  it("generates, stores and approves a trading signer with main-wallet signatures", async () => {
    vi.stubGlobal("fetch", infoRoutes());
    const address = await runtime.hyperliquid.storeApiWallet(user.id, "generate", "");
    expect(address).toMatch(/^0x[0-9a-f]{40}$/);

    let settings = await runtime.hyperliquid.settings(user.id);
    expect(settings?.apiWalletAddress).toBe(address);
    expect(settings?.apiWalletApprovedAt).toBeNull();
    expect(settings?.privateKeyCiphertext?.length).toBeGreaterThan(16);

    await expect(runtime.hyperliquid.storeApiWallet(user.id, "import", "not-a-key")).rejects.toThrow("Private key is invalid.");
    await expect(runtime.hyperliquid.storeApiWallet(user.id, "import", "")).rejects.toThrow("Private key is required.");

    await expect(runtime.hyperliquid.approveApiWallet(user, {
      action: { ...(await signAgentAction(address)).action, agentAddress: "0x4444444444444444444444444444444444444444" },
      signature: (await signAgentAction(address)).signature,
    })).rejects.toThrow("Invalid trading signer approval.");

    const signed = await signAgentAction(address);
    await runtime.hyperliquid.approveApiWallet(user, signed);
    settings = await runtime.hyperliquid.settings(user.id);
    expect(settings?.apiWalletApprovedAt).not.toBeNull();
    expect(settings?.apiWalletExpiresAt).toBeGreaterThan(Date.now());
    expect(runtime.hyperliquid.apiWalletIsReady(settings)).toBe(true);

    // Storing the identical signer address again must be refused.
    await runtime.hyperliquid.storeApiWallet(user.id, "import", signerPrivateKey);
    await expect(runtime.hyperliquid.storeApiWallet(user.id, "import", signerPrivateKey))
      .rejects.toThrow("Use a fresh trading signer address.");
  });

  it("rejects signer actions when Hyperliquid or the exchange is unreachable", async () => {
    vi.stubGlobal("fetch", hyperliquidFetch({ "/info": () => { throw new Error("down"); }, "/exchange": () => { throw new Error("down"); } }));
    const address = await runtime.hyperliquid.storeApiWallet(user.id, "generate", "");
    await expect(runtime.hyperliquid.approveApiWallet(user, await signAgentAction(address)))
      .rejects.toMatchObject({ message: "Hyperliquid exchange unavailable", status: 502 });
  });

  it("reconciles builder fee approvals against Hyperliquid's remote maximum", async () => {
    vi.stubGlobal("fetch", infoRoutes());
    const view = await runtime.hyperliquid.accountView(user);
    // A user who already approved the fee on Hyperliquid is approved on the
    // very first visit, before any local record exists.
    expect(view.builderFee).toEqual({ feeBps: 5, approved: true });
    expect(view.accountMode).toBe("unified");
    expect(view.accounts.map((account) => account.name)).toEqual(["Main Account", "vt-BTC Momentum"]);
    expect(view.accounts[0]?.balance).toBe("12.5");
    expect(view.totalBalance).toBe("112.75");

    await expect(runtime.hyperliquid.approveBuilderFee(user, await signBuilderFeeAction(11), 11)).rejects.toThrow("between 1 and 10 basis points");
    await expect(runtime.hyperliquid.approveBuilderFee(user, await signBuilderFeeAction(7), 3)).rejects.toThrow("Invalid builder fee approval.");
    await runtime.hyperliquid.approveBuilderFee(user, await signBuilderFeeAction(7), 7);
    expect((await runtime.hyperliquid.settings(user.id))?.builderFeeTenthsOfBp).toBe(70);
    expect((await runtime.hyperliquid.accountView(user)).builderFee).toEqual({ feeBps: 7, approved: true });

    await runtime.hyperliquid.cancelBuilderFee(user, await signBuilderFeeAction(0));
    expect((await runtime.hyperliquid.accountView(user)).builderFee).toEqual({ feeBps: 7, approved: false });
  });

  it("treats missing USDC as zero and only failed lookups as unavailable", async () => {
    vi.stubGlobal("fetch", infoRoutes({
      subAccounts: () => [
        { name: "vt-Funded", master: mainWallet.address, subAccountUser: "0x1111111111111111111111111111111111111111", spotState: { balances: [{ coin: "USDC", total: "100.25" }] } },
        { name: "vt-Empty", master: mainWallet.address, subAccountUser: "0x2222222222222222222222222222222222222222", spotState: { balances: [] } },
        // Hyperliquid omits spotState for never-funded sub-accounts.
        { name: "vt-NoSpot", master: mainWallet.address, subAccountUser: "0x3333333333333333333333333333333333333333" },
        { name: "vt-Negative", master: mainWallet.address, subAccountUser: "0x4444444444444444444444444444444444444444", spotState: { balances: [{ coin: "USDC", total: "-5" }] } },
      ],
    }));
    const view = await runtime.hyperliquid.accountView(user);
    expect(view.accounts.map((account) => [account.name, account.balance])).toEqual([
      ["Main Account", "12.5"],
      ["vt-Funded", "100.25"],
      ["vt-Empty", "0"],
      ["vt-NoSpot", "0"],
      ["vt-Negative", null],
    ]);
    expect(view.totalBalance).toBeNull();
  });

  it("reports unregistered wallets and lookup failures without exposing server details", async () => {
    vi.stubGlobal("fetch", infoRoutes({
      userRole: () => ({ role: "missing" }),
      spotClearinghouseState: () => { throw new Error("exchange down"); },
      subAccounts: () => { throw new Error("exchange down"); },
    }));
    const view = await runtime.hyperliquid.accountView(user);
    expect(view.accountMode).toBe("unregistered");
    expect(view.accountModeMessage).toContain("not registered");
    expect(view.accounts).toEqual([
      { name: "Main Account", address: mainWallet.address.toLowerCase(), agentId: null, agentName: null, balance: null },
    ]);
  });

  it("creates an agent sub-account with the stored signer and links it to the new agent", async () => {
    vi.stubGlobal("fetch", infoRoutes());
    await runtime.hyperliquid.storeApiWallet(user.id, "import", signerPrivateKey);
    await expect(runtime.hyperliquid.createAgentSubaccount(user, "BTC Momentum")).rejects.toThrow("Set up and approve a trading signer");

    await runtime.hyperliquid.approveApiWallet(user, await signAgentAction(signerAddress));

    const subaccount = await runtime.hyperliquid.createAgentSubaccount(user, "BTC Momentum");
    expect(subaccount.address).toBe("0x9999999999999999999999999999999999999999");

    const choices = await runtime.hyperliquid.tradingAccountChoices(user);
    expect(choices.apiWalletReady).toBe(true);
    expect(choices.subaccountCapacity).toBe("9 remaining of 10 total");
    expect(choices.subaccounts[0]?.name).toBe("vt-BTC Momentum");

    const agent = await runtime.agents.create(user.id, "BTC Momentum", randomUUID(), subaccount.address);
    expect(agent.tradingAccountAddress).toBe(subaccount.address);
    await expect(runtime.agents.create(user.id, "Second", randomUUID(), subaccount.address))
      .rejects.toMatchObject({ message: "That trading account is already assigned to another agent." });
    expect((await runtime.hyperliquid.accountView(user)).accounts[1]?.agentName).toBe("BTC Momentum");

    await expect(runtime.hyperliquid.createAgentSubaccount(user, "")).rejects.toThrow(HyperliquidError);
  });
});