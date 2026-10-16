import { HttpTransport } from "@nktkas/hyperliquid";
import { loadAgentDashboard, type AgentDashboard } from "./agent-dashboard.js";
import { LiveDashboards, type AccountFeedFactory } from "./live-dashboard.js";
import { agentId } from "../agents/agent-input.js";
import { and, eq, isNotNull, isNull } from "drizzle-orm";
import { privateKeyToAccount } from "viem/accounts";
import type { EncryptionKeySetting } from "../config.js";
import type { ApplicationDatabase } from "../db/shared-database.js";
import { hvAgents } from "../db/agent-schema.js";
import { hvHyperliquidSettings, type HyperliquidSettingsRow } from "../db/exchange-schema.js";
import {
  accountModeError,
  loadOwnedTradingAccounts,
  maxBuilderFee,
  type AccountMode,
  type OwnedTradingAccounts,
} from "./account-info.js";
import { addDecimals } from "./decimal.js";
import {
  agentSubaccountName,
  API_WALLET_APPROVAL_MS,
  BUILDER_FEE_BPS_TO_TENTHS,
  BUILDER_RECIPIENT,
  DEFAULT_BUILDER_FEE_BPS,
  MAX_BUILDER_FEE_BPS,
  MAX_BUILDER_FEE_TENTHS_OF_BP,
  MIN_BUILDER_FEE_BPS,
  REVOKED_BUILDER_FEE_RATE,
} from "./constants.js";
import { decryptSecret, encryptSecret, generateWalletPrivateKey, type EncryptionKey } from "./crypto.js";
import { exchangeErrorMessage, exchangeSuccess, relaySignedAction, SubaccountCreator } from "./exchange.js";
import { HyperliquidError } from "./errors.js";
import {
  parseSignedAction,
  signatureMatches,
  validApproveAgentAction,
  validApproveBuilderFeeAction,
  type SignedAction,
} from "./signing.js";

export interface AuthenticatedWallet {
  id: string;
  walletAddress: string;
}

export interface ApiWalletView {
  address: string | null;
  state: "not-setup" | "approval-required" | "approved" | "expired-or-unavailable";
  expiresAt: number | null;
  showExpired: boolean;
}

export interface BuilderFeeView {
  feeBps: number;
  approved: boolean;
}

export interface AccountRowView {
  name: string;
  address: string;
  agentId: string | null;
  agentName: string | null;
  /** Plain decimal USDC balance string, or null when unavailable. */
  balance: string | null;
}

export interface AccountView {
  walletAddress: string;
  accountMode: AccountMode;
  accountModeMessage: string | null;
  encryptionConfigured: boolean;
  apiWallet: ApiWalletView | null;
  builderFee: BuilderFeeView;
  accounts: AccountRowView[];
  /** Plain decimal total across accounts, or null when incomplete. */
  totalBalance: string | null;
  lookupError: string | null;
}

export interface SubaccountChoice {
  name: string | null;
  address: string;
  /** Plain decimal USDC balance string, or null when unavailable. */
  balance: string | null;
  assignedTo: { agentId: string; agentName: string } | null;
}

export interface TradingAccountChoices {
  mainAddress: string;
  mainBalance: string | null;
  mainAssignedTo: string | null;
  subaccounts: SubaccountChoice[];
  subaccountCapacity: string | null;
  lookupError: string | null;
  accountModeSupported: boolean;
  apiWalletReady: boolean;
}

/** Owns every user's Hyperliquid settings and the shared exchange clients. */
export class HyperliquidService {
  private readonly creator: SubaccountCreator;
  private readonly live: LiveDashboards;
  private started = false;
  private stopped = false;
  private registryTimer?: ReturnType<typeof setTimeout>;
  private registryTask?: Promise<void>;

  constructor(
    private readonly db: ApplicationDatabase,
    private readonly encryptionKey: EncryptionKeySetting | undefined,
    private readonly transport: HttpTransport = new HttpTransport(),
    feedFactory?: AccountFeedFactory,
  ) {
    this.creator = new SubaccountCreator(transport);
    this.live = new LiveDashboards(transport, feedFactory);
  }

  start() {
    if (this.started || this.stopped) return;
    this.started = true;
    const refresh = () => {
      this.registryTask = this.db.select({ address: hvAgents.tradingAccountAddress }).from(hvAgents)
        .where(isNotNull(hvAgents.tradingAccountAddress)).all()
        .then(async (rows) => { if (!this.stopped) await this.live.reconcile(rows.flatMap((row) => row.address ? [row.address] : [])); })
        .catch(() => { console.error("Unable to refresh linked Hyperliquid accounts; retrying"); })
        .finally(() => {
          if (!this.stopped) { this.registryTimer = setTimeout(refresh, 30_000); this.registryTimer.unref(); }
        });
    };
    refresh();
  }

  async close() {
    this.stopped = true;
    clearTimeout(this.registryTimer);
    await this.registryTask;
    await this.live.close();
  }

  private async dashboardAccount(userId: string, id: string) {
    const agent = await this.db.select().from(hvAgents).where(and(eq(hvAgents.id, agentId(id)), eq(hvAgents.userId, userId))).get();
    if (!agent) throw new HyperliquidError("Agent not found", 404);
    return agent.tradingAccountAddress;
  }

  async agentDashboard(userId: string, id: string) {
    const address = await this.dashboardAccount(userId, id);
    if (this.started && address) return this.live.snapshot(address);
    return loadAgentDashboard(this.transport, address);
  }

  async watchDashboard(userId: string, id: string, signal: AbortSignal, publish: (snapshot: AgentDashboard) => void) {
    const address = await this.dashboardAccount(userId, id);
    if (!this.started || !address) { publish(await loadAgentDashboard(this.transport, null)); return; }
    await this.live.watch(address, signal, publish);
  }

  async settings(userId: string): Promise<HyperliquidSettingsRow | undefined> {
    return this.db.select().from(hvHyperliquidSettings).where(eq(hvHyperliquidSettings.userId, userId)).get();
  }

  private async ensureSettings(userId: string): Promise<HyperliquidSettingsRow> {
    const now = Date.now();
    await this.db.insert(hvHyperliquidSettings).values({ userId, createdAt: now, updatedAt: now })
      .onConflictDoNothing({ target: hvHyperliquidSettings.userId }).run();
    const row = await this.settings(userId);
    if (!row) throw new HyperliquidError("Unable to store Hyperliquid settings", 503);
    return row;
  }

  /** True when the trading signer exists, is approved and not known-expired. */
  apiWalletIsReady(row: HyperliquidSettingsRow | undefined): boolean {
    if (!row?.apiWalletAddress || !row.privateKeyCiphertext || !row.privateKeyKeyId || !row.apiWalletApprovedAt) return false;
    const now = Date.now();
    if (row.apiWalletExpiresAt && row.apiWalletExpiresAt <= now) return false;
    if (row.apiWalletExpiresAt && !row.apiWalletExpiryCheckedAt) return false;
    return true;
  }

  private apiWalletView(row: HyperliquidSettingsRow | undefined): ApiWalletView | null {
    if (!row?.apiWalletAddress) return null;
    const expired = row.apiWalletExpiresAt !== null && row.apiWalletExpiresAt <= Date.now();
    const state: ApiWalletView["state"] = !row.apiWalletApprovedAt
      ? "approval-required"
      : expired || !this.apiWalletIsReady(row) ? "expired-or-unavailable" : "approved";
    return {
      address: row.apiWalletAddress,
      state,
      expiresAt: row.apiWalletExpiresAt,
      showExpired: Boolean(row.apiWalletApprovedAt) && expired,
    };
  }

  /** Generates or imports a trading signer and stores it encrypted. */
  async storeApiWallet(userId: string, source: "generate" | "import", importedKey: string): Promise<string> {
    if (!this.encryptionKey) throw new HyperliquidError("Trading signer storage is not configured on the server", 503);
    let privateKey: string;
    if (source === "generate") privateKey = generateWalletPrivateKey();
    else if (source === "import" && importedKey.trim()) privateKey = importedKey.trim();
    else if (source === "import") throw new HyperliquidError("Private key is required.");
    else throw new HyperliquidError("Select an API wallet option.");
    let address: string;
    try { address = privateKeyToAccount(privateKey as `0x${string}`).address.toLowerCase(); }
    catch { throw new HyperliquidError("Private key is invalid."); }
    const row = await this.ensureSettings(userId);
    if (row.apiWalletAddress === address) throw new HyperliquidError("Use a fresh trading signer address.");
    const ciphertext = encryptSecret(this.encryptionKey as EncryptionKey, privateKey);
    await this.db.update(hvHyperliquidSettings).set({
      apiWalletAddress: address,
      privateKeyCiphertext: Buffer.from(ciphertext),
      privateKeyKeyId: this.encryptionKey.id,
      apiWalletApprovedAt: null,
      apiWalletExpiresAt: null,
      apiWalletExpiryCheckedAt: null,
      updatedAt: Date.now(),
    }).where(eq(hvHyperliquidSettings.userId, userId)).run();
    return address;
  }

  /** Relays a main-wallet-signed ApproveAgent action and records the approval. */
  async approveApiWallet(user: AuthenticatedWallet, request: unknown): Promise<void> {
    const signed = parseSignedAction(request);
    const row = await this.settings(user.id);
    if (!row?.apiWalletAddress) throw new HyperliquidError("Set up a trading signer first.");
    if (!validApproveAgentAction(signed.action, row.apiWalletAddress)
      || !(await signatureMatches(signed.action, signed.signature, user.walletAddress, "approveAgent"))) {
      throw new HyperliquidError("Invalid trading signer approval.");
    }
    const exchange = await this.relay(signed);
    if (!exchangeSuccess(exchange)) {
      throw new HyperliquidError(exchangeErrorMessage(exchange), 502);
    }
    const now = Date.now();
    await this.db.update(hvHyperliquidSettings).set({
      apiWalletApprovedAt: now,
      apiWalletExpiresAt: now + API_WALLET_APPROVAL_MS,
      apiWalletExpiryCheckedAt: now,
      updatedAt: now,
    }).where(and(
      eq(hvHyperliquidSettings.userId, user.id),
      isNotNull(hvHyperliquidSettings.apiWalletAddress),
      isNotNull(hvHyperliquidSettings.privateKeyCiphertext),
      isNotNull(hvHyperliquidSettings.privateKeyKeyId),
    )).run();
    const approved = await this.settings(user.id);
    if (!approved?.apiWalletApprovedAt) throw new HyperliquidError("Trading signer approval conflict", 409);
  }

  /** Relays a main-wallet-signed ApproveBuilderFee action and stores the fee. */
  async approveBuilderFee(user: AuthenticatedWallet, request: unknown, feeBps: number): Promise<void> {
    const maxFeeRate = `${(feeBps / 100).toFixed(2)}%`;
    if (!Number.isSafeInteger(feeBps) || feeBps < MIN_BUILDER_FEE_BPS || feeBps > MAX_BUILDER_FEE_BPS) {
      throw new HyperliquidError("Builder fee must be between 1 and 10 basis points.");
    }
    await this.relayBuilderFee(user, request, maxFeeRate);
    await this.ensureSettings(user.id);
    await this.db.update(hvHyperliquidSettings).set({
      builderFeeTenthsOfBp: feeBps * BUILDER_FEE_BPS_TO_TENTHS,
      builderFeeApprovedAt: Date.now(),
      updatedAt: Date.now(),
    }).where(eq(hvHyperliquidSettings.userId, user.id)).run();
  }

  /** Relays a main-wallet-signed builder-fee revocation. */
  async cancelBuilderFee(user: AuthenticatedWallet, request: unknown): Promise<void> {
    await this.relayBuilderFee(user, request, REVOKED_BUILDER_FEE_RATE);
    await this.db.update(hvHyperliquidSettings).set({ builderFeeApprovedAt: null, updatedAt: Date.now() })
      .where(eq(hvHyperliquidSettings.userId, user.id)).run();
  }

  private async relayBuilderFee(user: AuthenticatedWallet, request: unknown, maxFeeRate: string): Promise<void> {
    const signed = parseSignedAction(request);
    if (!validApproveBuilderFeeAction(signed.action, maxFeeRate)
      || !(await signatureMatches(signed.action, signed.signature, user.walletAddress, "approveBuilderFee"))) {
      throw new HyperliquidError("Invalid builder fee approval.");
    }
    const exchange = await this.relay(signed);
    if (!exchangeSuccess(exchange)) {
      throw new HyperliquidError(exchangeErrorMessage(exchange), 502);
    }
  }

  private async relay(signed: SignedAction): Promise<unknown> {
    try {
      return await relaySignedAction(this.transport, signed.action, signed.signature);
    } catch {
      throw new HyperliquidError("Hyperliquid exchange unavailable", 502);
    }
  }

  /** Reconciles the stored builder fee with Hyperliquid's current maximum. */
  private async reconcileBuilderFee(user: AuthenticatedWallet): Promise<BuilderFeeView> {
    const row = await this.settings(user.id);
    const saved = row?.builderFeeTenthsOfBp ?? 0;
    try {
      const remote = await maxBuilderFee(this.transport, user.walletAddress, BUILDER_RECIPIENT);
      const capped = Math.min(remote, MAX_BUILDER_FEE_TENTHS_OF_BP);
      const tenths = capped > 0 && capped !== saved ? capped : saved;
      // Coverage must be judged against the value synced from Hyperliquid,
      // never against a stale pre-sync value: a first visit by a user who
      // already approved the fee on Hyperliquid must show as approved.
      const covered = tenths > 0 && remote >= tenths;
      if (tenths !== saved) {
        await this.ensureSettings(user.id);
        await this.db.update(hvHyperliquidSettings).set({ builderFeeTenthsOfBp: tenths, updatedAt: Date.now() })
          .where(eq(hvHyperliquidSettings.userId, user.id)).run();
      }
      if (covered && !row?.builderFeeApprovedAt) {
        await this.ensureSettings(user.id);
        await this.db.update(hvHyperliquidSettings).set({ builderFeeApprovedAt: Date.now(), updatedAt: Date.now() })
          .where(and(eq(hvHyperliquidSettings.userId, user.id), isNull(hvHyperliquidSettings.builderFeeApprovedAt))).run();
      } else if (!covered && row?.builderFeeApprovedAt) {
        await this.db.update(hvHyperliquidSettings).set({ builderFeeApprovedAt: null, updatedAt: Date.now() })
          .where(and(eq(hvHyperliquidSettings.userId, user.id), isNotNull(hvHyperliquidSettings.builderFeeApprovedAt))).run();
      }
      const feeBps = tenths > 0 && tenths % BUILDER_FEE_BPS_TO_TENTHS === 0
        ? Math.min(Math.max(tenths / BUILDER_FEE_BPS_TO_TENTHS, MIN_BUILDER_FEE_BPS), MAX_BUILDER_FEE_BPS)
        : DEFAULT_BUILDER_FEE_BPS;
      return { feeBps, approved: covered };
    } catch {
      // Keep the last known approval when the lookup fails.
      return { feeBps: this.displayFeeBps(saved), approved: Boolean(row?.builderFeeApprovedAt) };
    }
  }

  private displayFeeBps(tenths: number): number {
    return tenths > 0 && tenths % BUILDER_FEE_BPS_TO_TENTHS === 0
      ? Math.min(Math.max(tenths / BUILDER_FEE_BPS_TO_TENTHS, MIN_BUILDER_FEE_BPS), MAX_BUILDER_FEE_BPS)
      : DEFAULT_BUILDER_FEE_BPS;
  }

  /** Assembles the full Account page view for a signed-in wallet. */
  async accountView(user: AuthenticatedWallet): Promise<AccountView> {
    const [row, accounts, builderFee] = await Promise.all([
      this.settings(user.id),
      loadOwnedTradingAccounts(this.transport, user.walletAddress, { includeCapacity: false }),
      this.reconcileBuilderFee(user),
    ]);
    const assignments = await this.db.select({
      address: hvAgents.tradingAccountAddress,
      agentId: hvAgents.id,
      agentName: hvAgents.name,
    }).from(hvAgents).where(isNotNull(hvAgents.tradingAccountAddress)).all();
    const assignmentFor = (address: string) => assignments
      .find((assignment) => assignment.address?.toLowerCase() === address.toLowerCase());

    const rows: AccountRowView[] = [this.accountRow("Main Account", accounts.mainAddress, accounts.mainBalance, assignmentFor)];
    for (const subaccount of accounts.subaccounts) {
      rows.push(this.accountRow(subaccount.name ?? "Unnamed subaccount", subaccount.address, subaccount.balance, assignmentFor));
    }
    const total = rows.length > 1 ? this.totalBalance(accounts) : null;
    return {
      walletAddress: user.walletAddress,
      accountMode: accounts.accountMode,
      accountModeMessage: accountModeError(accounts.accountMode),
      encryptionConfigured: Boolean(this.encryptionKey),
      apiWallet: this.apiWalletView(row),
      builderFee,
      accounts: rows,
      totalBalance: total,
      lookupError: accounts.lookupError,
    };
  }

  private accountRow(name: string, address: string, balance: string | null,
    assignmentFor: (address: string) => { agentId: string; agentName: string } | undefined): AccountRowView {
    const assignment = assignmentFor(address);
    return {
      name,
      address,
      agentId: assignment?.agentId ?? null,
      agentName: assignment?.agentName ?? null,
      balance,
    };
  }

  private totalBalance(accounts: OwnedTradingAccounts): string | null {
    let total: string | null = "0";
    for (const balance of [accounts.mainBalance, ...accounts.subaccounts.map((subaccount) => subaccount.balance)]) {
      if (!balance) return null;
      total = addDecimals(total, balance);
      if (!total) return null;
    }
    return total;
  }

  /** Account choices for the Create Agent page. */
  async tradingAccountChoices(user: AuthenticatedWallet): Promise<TradingAccountChoices> {
    const [row, accounts] = await Promise.all([
      this.settings(user.id),
      loadOwnedTradingAccounts(this.transport, user.walletAddress, { includeCapacity: true }),
    ]);
    const assignments = await this.db.select({
      address: hvAgents.tradingAccountAddress,
      agentId: hvAgents.id,
      agentName: hvAgents.name,
    }).from(hvAgents).where(isNotNull(hvAgents.tradingAccountAddress)).all();
    const assignmentFor = (address: string) => assignments
      .find((assignment) => assignment.address?.toLowerCase() === address.toLowerCase());
    const mainAssignment = assignmentFor(accounts.mainAddress);
    return {
      mainAddress: accounts.mainAddress,
      mainBalance: accounts.mainBalance,
      mainAssignedTo: mainAssignment?.agentName ?? null,
      subaccounts: accounts.subaccounts.map((subaccount) => ({
        name: subaccount.name,
        address: subaccount.address,
        balance: subaccount.balance,
        assignedTo: assignmentFor(subaccount.address) ?? null,
      })),
      subaccountCapacity: accounts.subaccountCapacity,
      lookupError: accounts.lookupError ?? accountModeError(accounts.accountMode),
      accountModeSupported: accounts.accountMode === "unified",
      apiWalletReady: this.apiWalletIsReady(row),
    };
  }

  /** Creates a Hyperliquid sub-account for an agent using the trading signer. */
  async createAgentSubaccount(user: AuthenticatedWallet, displayName: string): Promise<{ address: string; created: boolean }> {
    const name = agentSubaccountName(displayName);
    if (!name || name === "vt-") throw new HyperliquidError("Enter an agent name first.");
    const privateKey = await this.decryptSigner(user.id);
    const result = await this.creator.create(user.walletAddress.toLowerCase(), name, privateKey);
    if (!result.ok) {
      throw new HyperliquidError(
        result.failure.kind === "rejected" ? result.failure.message
          : result.failure.kind === "lookup-unavailable" ? "Hyperliquid account data is temporarily unavailable."
            : "Sub-Account creation could not be confirmed. Refresh the account list before retrying.",
        result.failure.kind === "rejected" ? 422 : 502,
      );
    }
    return result.subaccount;
  }

  /** Loads the decrypted trading signer for exchange signing only. */
  async decryptSigner(userId: string): Promise<string> {
    if (!this.encryptionKey) throw new HyperliquidError("Trading signer storage is not configured on the server", 503);
    const row = await this.settings(userId);
    if (!row || !this.apiWalletIsReady(row)) throw new HyperliquidError("Set up and approve a trading signer first.", 409);
    if (!row.privateKeyCiphertext) throw new HyperliquidError("The stored trading signer is incomplete.", 409);
    if (row.privateKeyKeyId !== this.encryptionKey.id) {
      throw new HyperliquidError("The stored trading signer uses an unknown encryption key.", 409);
    }
    try {
      return decryptSecret(this.encryptionKey as EncryptionKey, new Uint8Array(row.privateKeyCiphertext));
    } catch {
      throw new HyperliquidError("The stored trading signer could not be decrypted.", 409);
    }
  }
}