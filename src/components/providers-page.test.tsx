// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { LoginView, ProviderSettings } from "../providers/provider-service.js";

const mocks = vi.hoisted(() => ({
  answerProviderLogin: vi.fn(), beginProviderLogin: vi.fn(), cancelProviderLogin: vi.fn(), disconnectProvider: vi.fn(),
  getProviderLogin: vi.fn(), getProviderSettings: vi.fn(), refreshProviderModels: vi.fn(),
}));
vi.mock("../web/providers.js", () => mocks);
import { ProvidersPage } from "./providers-page.js";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const initial: ProviderSettings = {
  storageReady: true, models: [], providers: [{ id: "test", name: "Test Provider", connected: false, configured: false, methods: [{ type: "api_key", label: "API key" }, { type: "oauth", label: "Subscription" }] }],
};
const attempt: LoginView = { id: "login", providerId: "test", state: "pending", events: [], prompt: { id: "prompt", input: { type: "secret", message: "Enter key" } } };

describe("Providers page", () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeAll(() => {
    // jsdom does not implement the native dialog lifecycle.
    const originalDialogMethods = Object.fromEntries(["showModal", "close"].map((key) => [key, Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, key)]));
    Object.defineProperties(HTMLDialogElement.prototype, {
      showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
      close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
    });
    afterAll(() => {
      for (const [key, descriptor] of Object.entries(originalDialogMethods)) {
        if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, key, descriptor);
        else Reflect.deleteProperty(HTMLDialogElement.prototype, key);
      }
    });
  });
  beforeEach(async () => {
    vi.resetAllMocks();
    vi.useFakeTimers();
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(<ProvidersPage initial={initial} />));
  });
  afterEach(async () => {
    await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.unstubAllGlobals();
  });
  const button = (text: string) => [...container.querySelectorAll("button")].find((button) => button.textContent === text)!;
  const dialog = () => container.querySelector<HTMLDialogElement>("dialog")!;

  it("shows onboarding without blocking access to existing account/history and disables storage without a key", () => {
    expect(container.textContent).toContain("before creating an agent");
    expect(container.textContent).toContain("Account settings and existing conversations remain accessible");
    expect(button("API key").className).toContain("cursor-pointer");
    expect(container.querySelector('[aria-labelledby="connected-providers-heading"]')).toBeNull();
    expect(container.textContent).not.toContain("Configured means");
    expect(container.textContent).not.toContain("For a Codex subscription");
    const markup = renderToStaticMarkup(<ProvidersPage initial={{ ...initial, storageReady: false }} />);
    expect(markup).toContain("AGENTS_ENCRYPTION_KEY");
    expect(markup).toContain('disabled=""');
  });

  it("keeps the page itself fixed and scrolls only the full provider list", () => {
    const page = container.firstElementChild!;
    expect(page.className).toContain("overflow-hidden");
    const list = container.querySelector('[aria-labelledby="all-providers-heading"] .overflow-y-auto');
    expect(list?.className).toContain("flex-1");
    // The connected section and search box live outside the scroller.
    expect(list?.contains(container.querySelector('[aria-label="Search providers"]'))).toBe(false);
  });

  it("bridges secret prompts without reflecting the answer and refreshes models after completion", async () => {
    mocks.beginProviderLogin.mockResolvedValue({ ok: true, data: attempt });
    await act(async () => button("API key").click());
    const input = dialog().querySelector<HTMLInputElement>("#provider-answer")!;
    expect(input.type).toBe("password");
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "private-key");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    mocks.answerProviderLogin.mockResolvedValue({ ok: true, data: { ...attempt, prompt: undefined } });
    await act(async () => dialog().querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(mocks.answerProviderLogin).toHaveBeenCalledWith({ data: { id: "login", promptId: "prompt", value: "private-key" } });
    expect(container.textContent).not.toContain("private-key");
    mocks.getProviderLogin.mockResolvedValue({ ok: true, data: { ...attempt, state: "complete", prompt: undefined } });
    mocks.getProviderSettings.mockResolvedValue({ ...initial, models: [{ provider: "test", modelId: "model", name: "Test model", providerName: "Test", thinkingLevels: [], contextWindow: 200_000, inputCost: 3, outputCost: 15, reasoning: true }] });
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(container.querySelector("dialog")).toBeNull();
    expect(container.querySelector('a[href="/agents/new"]')).toBeNull();
    expect(container.textContent).toContain("1 chat models");
  });

  it("keeps a failed sign-in open in the modal with its error", async () => {
    mocks.beginProviderLogin.mockResolvedValue({ ok: true, data: attempt });
    mocks.getProviderLogin.mockResolvedValue({ ok: true, data: { ...attempt, state: "failed", prompt: undefined, error: "Invalid key" } });
    mocks.getProviderSettings.mockResolvedValue(initial);
    await act(async () => button("API key").click());
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(dialog().textContent).toContain("Invalid key");
    await act(async () => [...dialog().querySelectorAll("button")].find((b) => b.textContent === "Close")!.click());
    expect(container.querySelector("dialog")).toBeNull();
  });

  it("shows connected providers above the full list, with green configured status", () => {
    const markup = document.createElement("div");
    markup.innerHTML = renderToStaticMarkup(<ProvidersPage initial={{ ...initial,
      providers: [
        { ...initial.providers[0]!, connected: true, configured: true, authType: "oauth" },
        { ...initial.providers[0]!, id: "broken", name: "Needs attention", connected: true },
        { ...initial.providers[0]!, id: "other", name: "Other Provider" },
      ],
      models: [{ provider: "test", modelId: "model", name: "Model", providerName: "Test", thinkingLevels: [], contextWindow: 200_000, inputCost: 3, outputCost: 15, reasoning: true }],
    }} />);
    const connected = markup.querySelector('[aria-labelledby="connected-providers-heading"]')!;
    const all = markup.querySelector('[aria-labelledby="all-providers-heading"]')!;
    expect(connected.querySelectorAll("h3")).toHaveLength(2);
    expect(connected.textContent).toContain("Needs attention");
    expect(connected.textContent).not.toContain("Other Provider");
    expect(connected.querySelector(".text-emerald-400")?.textContent).toContain("Configured");
    // Every row shows the provider logo, served lazily from the app's own cache.
    expect([...connected.querySelectorAll("img")].map((img) => img.getAttribute("src"))).toEqual(["/model-catalog/logos/test.svg", "/model-catalog/logos/broken.svg"]);
    expect(connected.querySelector("img")?.getAttribute("loading")).toBe("lazy");
    expect(all.querySelector("img")?.getAttribute("src")).toBe("/model-catalog/logos/other.svg");
    // Connected providers are not repeated in the full list.
    expect(all.querySelectorAll("h3")).toHaveLength(1);
    expect(all.textContent).toContain("Other Provider");
    expect(all.textContent).not.toContain("Needs attention");
    expect(markup.querySelector('a[href="/agents/new"]')).toBeNull();
  });

  it("labels reconnect and disconnect actions without repeating the provider name", () => {
    const markup = renderToStaticMarkup(<ProvidersPage initial={{ ...initial,
      providers: [{ ...initial.providers[0]!, connected: true, configured: true, authType: "oauth" }],
    }} />);
    const labels = [...markup.matchAll(/<button[^>]*>([^<]+)<\/button>/g)].map((match) => match[1]);
    expect(labels).toEqual(["Re-connect", "Re-connect", "Disconnect", "Refresh models"]);
    expect(markup).not.toContain("Replace:");
    expect(markup).not.toContain("Test Provider API key");
  });

  it("expands and collapses a provider's model list from the model count", async () => {
    const models = [{ provider: "test", modelId: "model", name: "Test model", providerName: "Test", thinkingLevels: [], contextWindow: 200_000, inputCost: 3, outputCost: 15, reasoning: true }];
    await act(async () => { await root.unmount(); root = createRoot(container); });
    await act(async () => root.render(<ProvidersPage initial={{ ...initial, models }} />));
    const toggle = () => [...container.querySelectorAll("button")].find((b) => b.textContent === "1 chat models")!;
    expect(toggle().getAttribute("aria-expanded")).toBe("false");
    await act(async () => toggle().click());
    expect(toggle().getAttribute("aria-expanded")).toBe("true");
    expect([...container.querySelectorAll("th")].map((th) => th.textContent)).toEqual(["Model", "ID", "Context", "Reasoning", "Input / 1M", "Output / 1M"]);
    expect(container.querySelector("tbody tr")?.textContent).toContain("Test model");
    expect(container.textContent).toContain("200K");
    expect(container.textContent).not.toContain("200K ctx");
    expect(container.textContent).toContain("Yes");
    expect([...container.querySelectorAll("tbody td")].map((td) => td.textContent)).toContain("3.00");
    expect([...container.querySelectorAll("tbody td")].map((td) => td.textContent)).toContain("15.00");
    expect([...container.querySelectorAll("td.font-mono")]).toHaveLength(2);
    await act(async () => toggle().click());
    expect(container.textContent).not.toContain("Test model");
  });

  it("confirms disconnects in a modal before disconnecting", async () => {
    mocks.disconnectProvider.mockResolvedValue({ ok: true });
    mocks.getProviderSettings.mockResolvedValue(initial);
    await act(async () => { await root.unmount(); root = createRoot(container); });
    await act(async () => root.render(<ProvidersPage initial={{ ...initial, providers: [{ ...initial.providers[0]!, connected: true, configured: true }] }} />));
    await act(async () => button("Disconnect").click());
    expect(mocks.disconnectProvider).not.toHaveBeenCalled();
    expect(dialog().textContent).toContain("Disconnect Test Provider?");
    await act(async () => [...dialog().querySelectorAll("button")].find((b) => b.textContent === "Cancel")!.click());
    expect(container.querySelector("dialog")).toBeNull();
    expect(mocks.disconnectProvider).not.toHaveBeenCalled();
    await act(async () => button("Disconnect").click());
    await act(async () => [...dialog().querySelectorAll("button")].find((b) => b.textContent === "Disconnect")!.click());
    expect(mocks.disconnectProvider).toHaveBeenCalledWith({ data: { providerId: "test" } });
  });

  it.each([true, false])("copies the exact device code and reports clipboard success=%s", async (success) => {
    const writeText = success ? vi.fn().mockResolvedValue(undefined) : vi.fn().mockRejectedValue(new Error("Clipboard denied"));
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    mocks.beginProviderLogin.mockResolvedValue({ ok: true, data: { ...attempt, prompt: undefined, events: [
      { type: "device_code", userCode: "ABCD-1234", verificationUri: "https://auth.example.test/device" },
    ] } });
    await act(async () => button("OAuth").click());
    expect([...dialog().querySelectorAll("button")].find((b) => b.textContent === "Copy to clipboard")!.className).toContain("cursor-pointer");
    await act(async () => [...dialog().querySelectorAll("button")].find((b) => b.textContent === "Copy to clipboard")!.click());
    expect(writeText).toHaveBeenCalledWith("ABCD-1234");
    expect(container.textContent).toContain(success ? "Copied!" : "Unable to copy. Select and copy the code above.");
    expect(container.textContent).toContain("ABCD-1234");
  });

  it("allows blank responses to optional native text prompts", async () => {
    const optional: LoginView = { ...attempt, prompt: { id: "domain", input: { type: "text", message: "Domain (blank for github.com)" } } };
    mocks.beginProviderLogin.mockResolvedValue({ ok: true, data: optional });
    mocks.answerProviderLogin.mockResolvedValue({ ok: true, data: { ...optional, prompt: undefined } });
    await act(async () => button("OAuth").click());
    expect(dialog().querySelector<HTMLInputElement>("#provider-answer")!.required).toBe(false);
    expect([...dialog().querySelectorAll("button")].find((b) => b.textContent === "Continue")!.disabled).toBe(false);
    await act(async () => dialog().querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(mocks.answerProviderLogin).toHaveBeenCalledWith({ data: { id: "login", promptId: "domain", value: "" } });
  });

  it("renders OAuth URLs/device codes safely and cancels an in-progress sign-in", async () => {
    const oauth: LoginView = { ...attempt, prompt: undefined, events: [
      { type: "auth_url", url: "javascript:alert(1)" },
      { type: "device_code", userCode: "ABCD-1234", verificationUri: "https://auth.example.test/device" },
    ] };
    mocks.beginProviderLogin.mockResolvedValue({ ok: true, data: oauth });
    await act(async () => button("OAuth").click());
    expect(container.innerHTML).not.toContain("javascript:");
    expect(dialog().textContent).toContain("ABCD-1234");
    expect(container.querySelector('a[href="https://auth.example.test/device"]')?.getAttribute("rel")).toContain("noopener");
    mocks.cancelProviderLogin.mockResolvedValue({ ok: true }); mocks.getProviderSettings.mockResolvedValue(initial);
    await act(async () => [...dialog().querySelectorAll("button")].find((b) => b.textContent === "Cancel sign-in")!.click());
    expect(mocks.cancelProviderLogin).toHaveBeenCalledWith({ data: { id: "login" } });
    expect(container.querySelector("dialog")).toBeNull();
  });
});