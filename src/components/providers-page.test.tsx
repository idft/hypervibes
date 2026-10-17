// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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

  it("bridges secret prompts without reflecting the answer and refreshes models after completion", async () => {
    mocks.beginProviderLogin.mockResolvedValue({ ok: true, data: attempt });
    await act(async () => button("API key").click());
    const input = container.querySelector<HTMLInputElement>("#provider-answer")!;
    expect(input.type).toBe("password");
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "private-key");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    mocks.answerProviderLogin.mockResolvedValue({ ok: true, data: { ...attempt, prompt: undefined } });
    await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(mocks.answerProviderLogin).toHaveBeenCalledWith({ data: { id: "login", promptId: "prompt", value: "private-key" } });
    expect(container.textContent).not.toContain("private-key");
    mocks.getProviderLogin.mockResolvedValue({ ok: true, data: { ...attempt, state: "complete", prompt: undefined } });
    mocks.getProviderSettings.mockResolvedValue({ ...initial, models: [{ provider: "test", modelId: "model", name: "Test model", providerName: "Test", thinkingLevels: [], contextWindow: 200_000, inputCost: 3, outputCost: 15, reasoning: true }] });
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(container.textContent).toContain("Provider connected.");
    expect(container.querySelector('a[href="/agents/new"]')).toBeNull();
    expect(container.textContent).toContain("1 chat models");
  });

  it("shows connected providers in matching rows above the full list, with green configured status", () => {
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
    expect(all.querySelectorAll("h3")).toHaveLength(3);
    expect(connected.querySelector("section")?.innerHTML).toBe(all.querySelector("section")?.innerHTML);
    expect(connected.querySelector(".text-emerald-400")?.textContent).toContain("Configured");
    expect(markup.querySelector('a[href="/agents/new"]')).toBeNull();
  });

  it.each([true, false])("copies the exact device code and reports clipboard success=%s", async (success) => {
    const writeText = success ? vi.fn().mockResolvedValue(undefined) : vi.fn().mockRejectedValue(new Error("Clipboard denied"));
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    mocks.beginProviderLogin.mockResolvedValue({ ok: true, data: { ...attempt, prompt: undefined, events: [
      { type: "device_code", userCode: "ABCD-1234", verificationUri: "https://auth.example.test/device" },
    ] } });
    await act(async () => button("Subscription").click());
    expect(button("Copy to clipboard").className).toContain("cursor-pointer");
    await act(async () => button("Copy to clipboard").click());
    expect(writeText).toHaveBeenCalledWith("ABCD-1234");
    expect(container.textContent).toContain(success ? "Copied!" : "Unable to copy. Select and copy the code above.");
    expect(container.textContent).toContain("ABCD-1234");
  });

  it("allows blank responses to optional native text prompts", async () => {
    const optional: LoginView = { ...attempt, prompt: { id: "domain", input: { type: "text", message: "Domain (blank for github.com)" } } };
    mocks.beginProviderLogin.mockResolvedValue({ ok: true, data: optional });
    mocks.answerProviderLogin.mockResolvedValue({ ok: true, data: { ...optional, prompt: undefined } });
    await act(async () => button("Subscription").click());
    expect(container.querySelector<HTMLInputElement>("#provider-answer")!.required).toBe(false);
    expect(button("Continue").disabled).toBe(false);
    await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(mocks.answerProviderLogin).toHaveBeenCalledWith({ data: { id: "login", promptId: "domain", value: "" } });
  });

  it("renders OAuth URLs/device codes safely and cancels an in-progress sign-in", async () => {
    const oauth: LoginView = { ...attempt, prompt: undefined, events: [
      { type: "auth_url", url: "javascript:alert(1)" },
      { type: "device_code", userCode: "ABCD-1234", verificationUri: "https://auth.example.test/device" },
    ] };
    mocks.beginProviderLogin.mockResolvedValue({ ok: true, data: oauth });
    await act(async () => button("Subscription").click());
    expect(container.innerHTML).not.toContain("javascript:");
    expect(container.textContent).toContain("ABCD-1234");
    expect(container.querySelector('a[href="https://auth.example.test/device"]')?.getAttribute("rel")).toContain("noopener");
    mocks.cancelProviderLogin.mockResolvedValue({ ok: true }); mocks.getProviderSettings.mockResolvedValue(initial);
    await act(async () => button("Cancel sign-in").click());
    expect(mocks.cancelProviderLogin).toHaveBeenCalledWith({ data: { id: "login" } });
    expect(container.querySelector('[aria-label="Provider sign-in"]')).toBeNull();
  });
});
