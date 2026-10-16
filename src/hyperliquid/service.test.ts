import { privateKeyToAccount } from "viem/accounts";
import { randomUUID } from "node:crypto";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { hvUsers } from "../db/auth-schema.js";
import { createRuntime, type AppRuntime } from "../runtime.js";
import { testConfig } from "../test-config.js";
import { BUILDER_RECIPIENT } from "./constants.js";
import { HyperliquidError } from "./errors.js";

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

  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hv-hyperliquid-"));
    runtime = await createRuntime(testConfig({
      APP_SQLITE_PATH: join(directory, "app.sqlite"),
      AGENTS_ENCRYPTION_KEY: encryptionKey,
    }));
    await runtime.db.insert(hvUsers).values({ id: user.id, name: "Owner", email: "owner@example.test" }).run();
  });
  afterEach(async () => {
    vi.unstubAllGlobals();
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
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