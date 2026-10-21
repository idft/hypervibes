// @vitest-environment jsdom
import { act } from "react";
import { type QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createTestQueryClient } from "../test/query.js";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LoginView, ProviderSettings } from "../providers/provider-service.js";

const mocks = vi.hoisted(() => ({
  answerProviderLogin: vi.fn(), beginProviderLogin: vi.fn(), cancelProviderLogin: vi.fn(), disconnectProvider: vi.fn(),
  getProviderSettings: vi.fn(), refreshProviderModels: vi.fn(), getSetupState: vi.fn(),
}));
vi.mock("../web/providers.js", () => mocks);
import { ProvidersPage as ProvidersPageComponent } from "./providers-page.js";
import { providerSettingsQueryOptions, setupQueryOptions } from "../web/provider-queries.js";

vi.mock("../web/setup.js", () => ({ getSetupState: mocks.getSetupState }));
function providerPage({ initial, agentCount = 0 }: { initial: ProviderSettings; agentCount?: number }, client = createTestQueryClient({ staleTime: Infinity })) {
  client.setQueryData(providerSettingsQueryOptions().queryKey, initial);
  client.setQueryData(setupQueryOptions().queryKey, { agentCount, providerConnected: false, apiKeyApproved: false, builderFeeApproved: false });
  return <QueryClientProvider client={client}><ProvidersPageComponent /></QueryClientProvider>;
}

const initial: ProviderSettings = {
  storageReady: true, models: [], providers: [{ id: "test", name: "Test Provider", connected: false, configured: false, methods: [{ type: "api_key", label: "API key" }, { type: "oauth", label: "Subscription" }] }],
};
class MockEventSource extends EventTarget {
  static instances: MockEventSource[] = [];
  static CLOSED = 2;
  readyState = 1;
  close = vi.fn();
  onerror: (() => void) | null = null;
  constructor(readonly url: string) { super(); MockEventSource.instances.push(this); }
  snapshot(view: LoginView) { this.dispatchEvent(new MessageEvent("provider-login", { data: JSON.stringify(view) })); }
}
const attempt: LoginView = { id: "login", providerId: "test", state: "pending", events: [], prompt: { id: "prompt", input: { type: "secret", message: "Enter key" } } };

describe("Providers page", () => {
  let container: HTMLDivElement;
  let root: Root, client: QueryClient;
  beforeEach(async () => {
    vi.resetAllMocks();
    client = createTestQueryClient({ staleTime: Infinity });
    mocks.getSetupState.mockResolvedValue({ agentCount: 0, providerConnected: false, apiKeyApproved: false, builderFeeApproved: false });
    vi.useFakeTimers();
    MockEventSource.instances = [];
    vi.stubGlobal("EventSource", MockEventSource);
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    await act(async () => root.render(providerPage({ initial }, client)));
  });
  afterEach(async () => {
    await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); vi.unstubAllGlobals();
  });
  const button = (text: string) => [...container.querySelectorAll("button")].find((button) => button.textContent === text)!;
  const dialog = () => container.querySelector<HTMLDialogElement>("dialog")!;

  it("shows onboarding without blocking access to existing account/history and disables storage without a key", () => {
    expect(container.textContent).toContain("Connect at least 1 AI provider");
    expect(container.querySelector('[aria-current="step"]')?.textContent).toContain("Connect at least 1 AI provider");
    expect(button("API key").className).toContain("cursor-pointer");
    expect(container.querySelector('[aria-labelledby="connected-providers-heading"]')).toBeNull();
    expect(container.textContent).not.toContain("Configured means");
    expect(container.textContent).not.toContain("For a Codex subscription");
    const markup = renderToStaticMarkup(providerPage({ initial: { ...initial, storageReady: false } }));
    expect(markup).toContain("AGENTS_ENCRYPTION_KEY");
    expect(markup).toContain('disabled=""');
  });

  it("links new users with a connected provider to Hyperliquid setup", () => {
    const settings = { ...initial, providers: [{ ...initial.providers[0]!, connected: true, configured: true }] };
    const markup = document.createElement("div");
    markup.innerHTML = renderToStaticMarkup(providerPage({ initial: settings }));
    const link = markup.querySelector('a[href="/account"]');
    expect(link?.textContent).toBe("Connect to Hyperliquid");
    expect(markup.querySelector('[aria-label="Initial setup"]')?.className).toContain("border-violet-800");
    expect(link?.parentElement?.getAttribute("aria-current")).toBe("step");
    expect(markup.textContent).toContain("Completed: Connect at least 1 AI provider");
    expect(renderToStaticMarkup(providerPage({ initial: settings, agentCount: 1 }))).not.toContain("Connect to Hyperliquid");
  });

  it("keeps the page itself fixed and scrolls only the full provider list", () => {
    const page = container.firstElementChild!;
    expect(page.className).toContain("overflow-hidden");
    const list = container.querySelector('[aria-labelledby="all-providers-heading"] .overflow-y-auto');
    expect(list?.className).toContain("flex-1");
    // The connected section and search box live outside the scroller.
    expect(list?.contains(container.querySelector('[aria-label="Search providers"]'))).toBe(false);
  });

  it("retains the provider snapshot after refresh failure and recovers on retry", async () => {
    mocks.refreshProviderModels.mockResolvedValue(undefined);
    mocks.getProviderSettings.mockRejectedValueOnce(new Error("Unable to refresh providers"));
    await act(async () => button("Refresh models").click());
    expect(container.textContent).toContain("Test Provider");
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Unable to refresh providers");
    expect(button("Refresh models").disabled).toBe(false);
    mocks.getProviderSettings.mockResolvedValue(initial);
    await act(async () => button("Refresh models").click());
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it("reports a rejected login mutation and allows retry without opening a stream", async () => {
    mocks.beginProviderLogin.mockRejectedValueOnce(new Error("Another sign-in is in progress"));
    mocks.getProviderSettings.mockResolvedValue(initial);
    await act(async () => button("API key").click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Another sign-in is in progress");
    expect(MockEventSource.instances).toHaveLength(0);
    mocks.beginProviderLogin.mockResolvedValue(attempt);
    await act(async () => button("API key").click());
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(MockEventSource.instances).toHaveLength(1);
  });

  it("bridges secret prompts without reflecting the answer and refreshes models after completion", async () => {
    mocks.beginProviderLogin.mockResolvedValue(attempt);
    await act(async () => button("API key").click());
    const input = dialog().querySelector<HTMLInputElement>("#provider-answer")!;
    expect(input.type).toBe("password");
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "private-key");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    mocks.answerProviderLogin.mockResolvedValue(undefined);
    await act(async () => dialog().querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(mocks.answerProviderLogin).toHaveBeenCalledWith({ data: { id: "login", promptId: "prompt", value: "private-key" } });
    expect(container.textContent).not.toContain("private-key");
    mocks.getProviderSettings.mockResolvedValue({ ...initial, models: [{ provider: "test", modelId: "model", name: "Test model", providerName: "Test", thinkingLevels: [], contextWindow: 200_000, inputCost: 3, outputCost: 15, reasoning: true }] });
    const source = MockEventSource.instances[0]!;
    expect(source.url).toBe("/api/providers/logins/login/events");
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    expect(mocks.getProviderSettings).not.toHaveBeenCalled();
    await act(async () => source.snapshot({ ...attempt, state: "complete", prompt: undefined }));
    expect(source.close).toHaveBeenCalled();
    expect(container.querySelector("dialog")).toBeNull();
    expect(container.querySelector('a[href="/agents/new"]')?.closest('[aria-label="Initial setup"]')).not.toBeNull();
    expect(container.textContent).toContain("1 chat models");
  });

  it("receives prompt updates and closes the stream on cancel or unmount", async () => {
    mocks.beginProviderLogin.mockResolvedValue({ ...attempt, prompt: undefined });
    await act(async () => button("OAuth").click());
    const source = MockEventSource.instances[0]!;
    await act(async () => source.snapshot(attempt));
    expect(dialog().querySelector<HTMLInputElement>("#provider-answer")?.type).toBe("password");
    await act(async () => source.onerror?.());
    expect(container.textContent).toContain("Reconnecting");
    await act(async () => source.snapshot(attempt));
    expect(container.textContent).not.toContain("Reconnecting");
    expect(MockEventSource.instances).toHaveLength(1);
    mocks.cancelProviderLogin.mockResolvedValue(undefined);
    mocks.getProviderSettings.mockResolvedValue(initial);
    await act(async () => [...dialog().querySelectorAll("button")].find((b) => b.textContent === "Cancel sign-in")!.click());
    expect(source.close).toHaveBeenCalled();
    await act(async () => button("API key").click());
    const reopened = MockEventSource.instances[1]!;
    await act(async () => root.render(<div />));
    expect(reopened.close).toHaveBeenCalled();
  });

  it("keeps a failed sign-in open in the modal with its error", async () => {
    mocks.beginProviderLogin.mockResolvedValue(attempt);
    mocks.getProviderSettings.mockResolvedValue(initial);
    await act(async () => button("API key").click());
    await act(async () => MockEventSource.instances[0]!.snapshot({ ...attempt, state: "failed", prompt: undefined, error: "Invalid key" }));
    expect(dialog().textContent).toContain("Invalid key");
    await act(async () => [...dialog().querySelectorAll("button")].find((b) => b.textContent === "Close")!.click());
    expect(container.querySelector("dialog")).toBeNull();
  });

  it("shows connected providers above the full list, with green configured status", () => {
    const markup = document.createElement("div");
    markup.innerHTML = renderToStaticMarkup(providerPage({ initial: { ...initial,
      providers: [
        { ...initial.providers[0]!, connected: true, configured: true, authType: "oauth" },
        { ...initial.providers[0]!, id: "broken", name: "Needs attention", connected: true },
        { ...initial.providers[0]!, id: "other", name: "Other Provider" },
      ],
      models: [{ provider: "test", modelId: "model", name: "Model", providerName: "Test", thinkingLevels: [], contextWindow: 200_000, inputCost: 3, outputCost: 15, reasoning: true }],
    } }));
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
    expect(markup.querySelector('a[href="/agents/new"]')?.closest('[aria-label="Initial setup"]')).not.toBeNull();
  });

  it("labels reconnect and disconnect actions without repeating the provider name", () => {
    const markup = renderToStaticMarkup(providerPage({ initial: { ...initial,
      providers: [{ ...initial.providers[0]!, connected: true, configured: true, authType: "oauth" }],
    } }));
    const labels = [...markup.matchAll(/<button[^>]*>([^<]+)<\/button>/g)].map((match) => match[1]);
    expect(labels).toEqual(["Re-connect", "Re-connect", "Disconnect", "Refresh models"]);
    expect(markup).not.toContain("Replace:");
    expect(markup).not.toContain("Test Provider API key");
  });

  it("expands and collapses a provider's model list from the model count", async () => {
    const models = [{ provider: "test", modelId: "model", name: "Test model", providerName: "Test", thinkingLevels: [], contextWindow: 200_000, inputCost: 3, outputCost: 15, reasoning: true }];
    await act(async () => { await root.unmount(); root = createRoot(container); });
    await act(async () => root.render(providerPage({ initial: { ...initial, models } }, client)));
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
    mocks.disconnectProvider.mockResolvedValue(undefined);
    mocks.getProviderSettings.mockResolvedValue(initial);
    await act(async () => { await root.unmount(); root = createRoot(container); });
    await act(async () => root.render(providerPage({ initial: { ...initial, providers: [{ ...initial.providers[0]!, connected: true, configured: true }] } }, client)));
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
    mocks.beginProviderLogin.mockResolvedValue({ ...attempt, prompt: undefined, events: [
      { type: "device_code", userCode: "ABCD-1234", verificationUri: "https://auth.example.test/device" },
    ] });
    await act(async () => button("OAuth").click());
    expect([...dialog().querySelectorAll("button")].find((b) => b.textContent === "Copy to clipboard")!.className).toContain("cursor-pointer");
    await act(async () => [...dialog().querySelectorAll("button")].find((b) => b.textContent === "Copy to clipboard")!.click());
    expect(writeText).toHaveBeenCalledWith("ABCD-1234");
    expect(container.textContent).toContain(success ? "Copied!" : "Unable to copy. Select and copy the code above.");
    expect(container.textContent).toContain("ABCD-1234");
  });

  it("allows blank responses to optional native text prompts", async () => {
    const optional: LoginView = { ...attempt, prompt: { id: "domain", input: { type: "text", message: "Domain (blank for github.com)" } } };
    mocks.beginProviderLogin.mockResolvedValue(optional);
    mocks.answerProviderLogin.mockResolvedValue(undefined);
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
    mocks.beginProviderLogin.mockResolvedValue(oauth);
    await act(async () => button("OAuth").click());
    expect(container.innerHTML).not.toContain("javascript:");
    expect(dialog().textContent).toContain("ABCD-1234");
    expect(container.querySelector('a[href="https://auth.example.test/device"]')?.getAttribute("rel")).toContain("noopener");
    mocks.cancelProviderLogin.mockResolvedValue(undefined); mocks.getProviderSettings.mockResolvedValue(initial);
    await act(async () => [...dialog().querySelectorAll("button")].find((b) => b.textContent === "Cancel sign-in")!.click());
    expect(mocks.cancelProviderLogin).toHaveBeenCalledWith({ data: { id: "login" } });
    expect(container.querySelector("dialog")).toBeNull();
  });
});