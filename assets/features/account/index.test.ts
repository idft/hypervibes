import { afterEach, describe, expect, it, vi } from "vitest";

const wallet = vi.hoisted(() => ({
  requestAddresses: vi.fn(async () => ["0x1111111111111111111111111111111111111111"]),
  signTypedData: vi.fn(async () => "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1b"),
}));

vi.mock("viem", () => ({
  createWalletClient: vi.fn(() => wallet),
  custom: vi.fn((provider) => provider),
}));

import { initAccount } from "./index";

function render(autoPrompt: boolean, walletAddress = "0x1111111111111111111111111111111111111111") {
  document.body.innerHTML = `
    <div data-account-page data-referral-auto-prompt="${autoPrompt}" data-referral-wallet-address="${walletAddress}">
      <button data-referral-open></button>
      <div data-referral-modal class="hidden"><button data-referral-close></button><button data-referral-dismiss></button><button data-referral-claim></button><p data-referral-status></p></div>
    </div>
  `;
}

async function flush() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

afterEach(() => {
  document.body.replaceChildren();
  window.localStorage.clear();
  delete (window as Window & { ethereum?: unknown }).ethereum;
  vi.restoreAllMocks();
  wallet.requestAddresses.mockClear();
  wallet.signTypedData.mockClear();
});

describe("referral discount", () => {
  it("automatically opens only when the server marks the approved offer available", () => {
    render(false);
    initAccount();
    expect(document.querySelector("[data-referral-modal]")?.classList.contains("hidden")).toBe(true);

    document.body.replaceChildren();
    render(true);
    initAccount();
    expect(document.querySelector("[data-referral-modal]")?.classList.contains("hidden")).toBe(false);
  });

  it("stores dismissal per wallet and still permits manual reopening", () => {
    render(true);
    window.localStorage.setItem("hypervibes:referral-prompt-dismissed:0x2222222222222222222222222222222222222222", "true");
    initAccount();
    const modal = document.querySelector<HTMLElement>("[data-referral-modal]");
    if (!modal) throw new Error("Referral modal was not rendered");
    expect(modal.classList.contains("hidden")).toBe(false);

    document.querySelector<HTMLElement>("[data-referral-dismiss]")?.click();
    expect(window.localStorage.getItem("hypervibes:referral-prompt-dismissed:0x1111111111111111111111111111111111111111")).toBe("true");
    expect(modal.classList.contains("hidden")).toBe(true);

    document.querySelector<HTMLElement>("[data-referral-open]")?.click();
    expect(modal.classList.contains("hidden")).toBe(false);
  });

  it("requests a signing payload before relaying the signed claim and shows server errors", async () => {
    render(false);
    Object.defineProperty(window, "ethereum", { configurable: true, value: { request: vi.fn() } });
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({
        action: { type: "setReferrer", code: "HYPERVIBES" }, nonce: 1,
        domain: { name: "Exchange", version: "1", chainId: 1337, verifyingContract: "0x0000000000000000000000000000000000000000" },
        primaryType: "Agent", types: { Agent: [{ name: "source", type: "string" }, { name: "connectionId", type: "bytes32" }] },
        message: { source: "a", connectionId: "0x0000000000000000000000000000000000000000000000000000000000000000" },
      }), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ error: "Referral eligibility changed." }), { status: 409 }));
    vi.stubGlobal("fetch", fetchMock);
    initAccount();
    document.querySelector<HTMLElement>("[data-referral-open]")?.click();
    document.querySelector<HTMLElement>("[data-referral-claim]")?.click();
    await flush();
    await flush();

    expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
      "/account/referral/signing-payload",
      "/account/referral/claim",
    ]);
    expect(wallet.signTypedData).toHaveBeenCalledOnce();
    expect(document.querySelector("[data-referral-status]")?.textContent).toBe("Referral eligibility changed.");
  });
});
