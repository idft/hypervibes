import { afterEach, describe, expect, it, vi } from "vitest";

import { initAccount } from "./index";

function render(autoPrompt: boolean, walletAddress = "0x1111111111111111111111111111111111111111") {
  document.body.innerHTML = `
    <div data-account-page data-referral-auto-prompt="${autoPrompt}" data-referral-wallet-address="${walletAddress}">
      <button data-referral-open></button>
      <div data-referral-modal class="hidden"><button data-referral-close></button><button data-referral-dismiss></button><a data-referral-claim href="https://app.hyperliquid.xyz/join/HYPERVIBES" target="_blank" rel="noopener noreferrer">Claim on Hyperliquid</a></div>
    </div>
  `;
}

afterEach(() => {
  document.body.replaceChildren();
  window.localStorage.clear();
  delete (window as Window & { ethereum?: unknown }).ethereum;
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("referral discount", () => {
  it("automatically opens only when the server marks the offer available", () => {
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

  it("opens Hyperliquid's referral link without signing or submitting a claim in the app", () => {
    render(true);
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    initAccount();
    const link = document.querySelector<HTMLAnchorElement>("[data-referral-claim]");
    if (!link) throw new Error("Referral link was not rendered");
    link.addEventListener("click", (event) => event.preventDefault());
    link.click();

    expect(link.href).toBe("https://app.hyperliquid.xyz/join/HYPERVIBES");
    expect(link.target).toBe("_blank");
    expect(link.rel).toBe("noopener noreferrer");
    expect(fetchMock).not.toHaveBeenCalled();
    expect(window.localStorage.getItem("hypervibes:referral-prompt-dismissed:0x1111111111111111111111111111111111111111")).toBe("true");
    expect(document.querySelector("[data-referral-modal]")?.classList.contains("hidden")).toBe(true);
  });
});
