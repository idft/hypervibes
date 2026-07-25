import { createWalletClient, custom } from "viem";
import { csrfToken } from "../../core/csrf";

const CHAIN_ID = 42161;
const CHAIN_HEX = "0xa4b1";
type WalletProvider = Parameters<typeof custom>[0];

function walletClient() {
  const provider = (window as Window & { ethereum?: WalletProvider }).ethereum;
  if (!provider) throw new Error("No Ethereum wallet found.");
  return { client: createWalletClient({ transport: custom(provider) }), provider };
}

async function selectSignatureChain(provider: WalletProvider) {
  try {
    await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: CHAIN_HEX }] });
  } catch (reason: unknown) {
    if (!(typeof reason === "object" && reason !== null && "code" in reason && reason.code === 4902)) {
      throw new Error("Switch your wallet to Arbitrum Mainnet to sign Hyperliquid approvals.", { cause: reason });
    }
    await provider.request({ method: "wallet_addEthereumChain", params: [{ chainId: CHAIN_HEX, chainName: "Arbitrum One", nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 }, rpcUrls: ["https://arb1.arbitrum.io/rpc"], blockExplorerUrls: ["https://arbiscan.io"] }] });
    await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: CHAIN_HEX }] });
  }
}

async function signWalletAction(primaryType: string, types: Record<string, readonly { name: string; type: string }[]>, action: Record<string, string | number | boolean>) {
  const { client, provider } = walletClient();
  await selectSignatureChain(provider);
  const [account] = await client.requestAddresses();
  if (!account) throw new Error("No wallet account selected.");
  const signature = await client.signTypedData({ account, domain: { name: "HyperliquidSignTransaction", version: "1", chainId: CHAIN_ID, verifyingContract: "0x0000000000000000000000000000000000000000" }, types, primaryType, message: action });
  return { action, signature };
}

function actionBase() {
  return { hyperliquidChain: "Mainnet", signatureChainId: CHAIN_HEX, nonce: Date.now() };
}

function jsonHeaders() {
  return { "content-type": "application/json", "X-CSRF-Token": csrfToken() ?? "" };
}

async function approveTradingSigner(page: HTMLElement) {
  const status = page.querySelector<HTMLElement>("[data-api-wallet-status]");
  const address = page.dataset.apiWalletAddress;
  if (!status || !address) throw new Error("Set up a trading signer first.");
  status.textContent = "Awaiting wallet signature...";
  const signed = await signWalletAction("HyperliquidTransaction:ApproveAgent", { "HyperliquidTransaction:ApproveAgent": [{ name: "hyperliquidChain", type: "string" }, { name: "agentAddress", type: "address" }, { name: "agentName", type: "string" }, { name: "nonce", type: "uint64" }] }, { type: "approveAgent", ...actionBase(), agentAddress: address, agentName: "Vibetrading" });
  const response = await fetch("/account/approve-api-wallet", { method: "POST", headers: jsonHeaders(), body: JSON.stringify(signed) });
  if (!response.ok) throw new Error("Hyperliquid did not approve the trading signer.");
  window.location.reload();
}

async function generateAndApproveSigner(source: "generate" | "import", privateKey: HTMLInputElement | null, force: boolean) {
  const page = document.querySelector<HTMLElement>("[data-account-page]");
  const status = page?.querySelector<HTMLElement>("[data-api-wallet-status]");
  if (!page || !status) return;
  try {
    if (force || !page.dataset.apiWalletAddress) {
      status.textContent = force ? "Re-generating trading signer..." : "Generating trading signer...";
      const response = await fetch("/account/api-wallet", { method: "POST", headers: jsonHeaders(), body: JSON.stringify({ walletSource: source, hyperliquidPrivateKey: source === "import" ? privateKey?.value ?? "" : "" }) });
      const result = (await response.json().catch(() => null)) as { api_wallet_address?: string; error?: string } | null;
      if (!response.ok || !result?.api_wallet_address) throw new Error(result?.error ?? "Could not generate trading signer.");
      page.dataset.apiWalletAddress = result.api_wallet_address;
    }
    await approveTradingSigner(page);
  } catch (reason) {
    status.textContent = reason instanceof Error ? reason.message : "Trading signer setup failed.";
  }
}

function initApiWallet(page: HTMLElement) {
  const form = page.querySelector<HTMLFormElement>("[data-api-wallet-form]");
  const submit = form?.querySelector<HTMLButtonElement>("[data-api-wallet-submit]");
  const toggle = form?.querySelector<HTMLButtonElement>("[data-api-wallet-import-toggle]");
  const importSection = page.querySelector<HTMLElement>("[data-api-wallet-import]");
  const privateKey = importSection?.querySelector<HTMLInputElement>('input[name="hyperliquid_private_key"]');
  if (form && submit && toggle && importSection && privateKey && form.dataset.bound !== "true") {
    form.dataset.bound = "true";
    let source: "generate" | "import" = "generate";
    toggle.addEventListener("click", () => {
      const importing = importSection.classList.contains("hidden");
      importSection.classList.toggle("hidden", !importing);
      source = importing ? "import" : "generate";
      submit.textContent = importing ? "Import API Key" : "Generate API Key";
      toggle.textContent = importing ? "Generate API key" : "Import existing API key";
      privateKey.required = importing;
      if (!importing) privateKey.value = "";
    });
    submit.addEventListener("click", () => void generateAndApproveSigner(source, privateKey, false));
  }
  const regenerate = page.querySelector<HTMLButtonElement>("[data-api-wallet-regenerate]");
  if (regenerate && regenerate.dataset.bound !== "true") {
    regenerate.dataset.bound = "true";
    regenerate.addEventListener("click", () => void generateAndApproveSigner("generate", null, true));
  }
}

function initBuilderFee(page: HTMLElement) {
  const slider = page.querySelector<HTMLInputElement>("[data-builder-fee]");
  const input = page.querySelector<HTMLInputElement>("[data-builder-fee-input]");
  const status = page.querySelector<HTMLElement>("[data-builder-fee-status]");
  if (!slider || !input || slider.dataset.bound === "true") return;
  slider.dataset.bound = "true";
  const clamp = (value: number) => Math.max(Number(slider.min), Math.min(Number(slider.max), Math.round(value)));
  const syncSlider = () => { input.value = (Number(slider.value) / 100).toFixed(2); };
  slider.addEventListener("input", syncSlider);
  input.addEventListener("input", () => { if (Number.isFinite(Number(input.value))) slider.value = String(clamp(Number(input.value) * 100)); });
  input.addEventListener("blur", syncSlider);
  const submit = async (maxFeeRate: string) => {
    if (!status) return;
    status.textContent = "Awaiting wallet signature...";
    const signed = await signWalletAction("HyperliquidTransaction:ApproveBuilderFee", { "HyperliquidTransaction:ApproveBuilderFee": [{ name: "hyperliquidChain", type: "string" }, { name: "maxFeeRate", type: "string" }, { name: "builder", type: "address" }, { name: "nonce", type: "uint64" }] }, { type: "approveBuilderFee", ...actionBase(), maxFeeRate, builder: page.dataset.builderRecipient ?? "" });
    const response = await fetch(maxFeeRate === "0.00%" ? "/account/cancel-builder-fee" : "/account/approve-builder-fee", { method: "POST", headers: jsonHeaders(), body: JSON.stringify(maxFeeRate === "0.00%" ? signed : { ...signed, feeBps: clamp(Number(input.value) * 100) }) });
    if (!response.ok) throw new Error("Hyperliquid did not update the fee approval.");
    window.location.assign(((await response.json().catch(() => null)) as { redirect?: string } | null)?.redirect ?? "/account");
  };
  page.querySelector<HTMLButtonElement>("[data-approve-builder-fee]")?.addEventListener("click", () => void submit(`${(clamp(Number(input.value) * 100) / 100).toFixed(2)}%`).catch((reason: unknown) => { if (status) status.textContent = reason instanceof Error ? reason.message : "Approval failed."; }));
  page.querySelector<HTMLButtonElement>("[data-cancel-builder-fee]")?.addEventListener("click", () => void submit("0.00%").catch((reason: unknown) => { if (status) status.textContent = reason instanceof Error ? reason.message : "Cancellation failed."; }));
}

function canonicalTransferAmount(value: string): string | null {
  if (!/^\d+(?:\.\d+)?$/.test(value) || value.includes("e") || value.includes("E")) return null;
  const [integer, fraction = ""] = value.split(".");
  if (fraction.length > 8) return null;
  const normalizedInteger = integer.replace(/^0+(?=\d)/, "");
  const normalizedFraction = fraction.replace(/0+$/, "");
  if (BigInt(normalizedInteger) === 0n && !normalizedFraction) return null;
  return normalizedFraction ? `${normalizedInteger}.${normalizedFraction}` : normalizedInteger;
}

function initTransfers(page: HTMLElement) {
  const form = page.querySelector<HTMLFormElement>("[data-transfer-form]");
  const from = form?.querySelector<HTMLSelectElement>("[data-transfer-from]");
  const to = form?.querySelector<HTMLSelectElement>("[data-transfer-to]");
  const amount = form?.querySelector<HTMLInputElement>("[data-transfer-amount]");
  const submit = form?.querySelector<HTMLButtonElement>("[data-transfer-submit]");
  const status = form?.querySelector<HTMLElement>("[data-transfer-status]");
  if (!form || !from || !to || !amount || !submit || !status || form.dataset.bound === "true") return;
  form.dataset.bound = "true";
  const update = () => { submit.disabled = page.dataset.transfersEnabled !== "true" || from.value === to.value || canonicalTransferAmount(amount.value) === null; };
  from.addEventListener("change", update); to.addEventListener("change", update); amount.addEventListener("input", update); update();
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const value = canonicalTransferAmount(amount.value);
    if (!value || from.value === to.value) { status.textContent = "Choose two different accounts and enter a positive USDC amount with up to 8 decimals."; update(); return; }
    submit.disabled = true; status.textContent = "Awaiting wallet signature...";
    const action = { type: "sendAsset", ...actionBase(), destination: to.value, sourceDex: "spot", destinationDex: "spot", token: "USDC:0x6d1e7cde53ba9467b783cb7c530ce054", amount: value, fromSubAccount: from.value === page.dataset.mainAddress ? "" : from.value };
    void signWalletAction("HyperliquidTransaction:SendAsset", { "HyperliquidTransaction:SendAsset": [{ name: "hyperliquidChain", type: "string" }, { name: "destination", type: "string" }, { name: "sourceDex", type: "string" }, { name: "destinationDex", type: "string" }, { name: "token", type: "string" }, { name: "amount", type: "string" }, { name: "fromSubAccount", type: "string" }, { name: "nonce", type: "uint64" }] }, action).then(async (signed) => { status.textContent = "Submitting transfer..."; const response = await fetch("/account/transfers", { method: "POST", headers: jsonHeaders(), body: JSON.stringify(signed) }); if (!response.ok) throw new Error("Transfer failed."); window.location.reload(); }).catch((reason: unknown) => { status.textContent = reason instanceof Error ? reason.message : "Transfer failed."; update(); });
  });
}

function blockieDataUri(address: string): string {
  let seed = 0;
  for (const character of address) seed = ((seed << 5) - seed + character.charCodeAt(0)) >>> 0;
  const random = () => { seed = (seed * 9301 + 49297) % 233280; return seed / 233280; };
  const color = () => `hsl(${Math.floor(random() * 360)} ${Math.floor(random() * 60 + 40)}% ${Math.floor((random() + random() + random() + random()) * 25)}%)`;
  const foreground = color(); const background = color(); const spot = color(); const squares: string[] = [];
  for (let row = 0; row < 8; row += 1) { const values = Array.from({ length: 4 }, () => Math.floor(random() * 2.3)); values.push(...values.slice().reverse()); values.forEach((value, column) => { if (value) squares.push(`<rect x="${column}" y="${row}" width="1" height="1" fill="${value === 1 ? foreground : spot}"/>`); }); }
  return `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8" fill="${background}"/>${squares.join("")}</svg>`)}`;
}

export function initAccount(root: ParentNode = document) {
  root.querySelectorAll<HTMLElement>("[data-account-page]").forEach((page) => { initApiWallet(page); initBuilderFee(page); initTransfers(page); });
  const address = document.querySelector<HTMLElement>("[data-account-page]")?.dataset.accountAddress ?? document.querySelector<HTMLElement>("[data-account-navbar]")?.dataset.accountAddress;
  if (!address) return;
  document.querySelectorAll<HTMLImageElement>("[data-account-identicon]").forEach((image) => { image.src = blockieDataUri(address); image.classList.remove("hidden"); });
  const label = document.querySelector<HTMLElement>("[data-account-navbar-label]");
  if (label && address.length > 10) label.textContent = `${address.slice(0, 6)}...${address.slice(-4)}`;
}
