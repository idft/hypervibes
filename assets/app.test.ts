import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./core/htmx", () => ({}));
vi.mock("./core/csrf", () => ({ installCsrf: vi.fn(), seedCsrfTokens: vi.fn() }));
vi.mock("./features/account", () => ({ initAccount: vi.fn() }));
vi.mock("./features/agents/page", () => ({ initAgentPage: vi.fn(), installAgentPageLifecycle: vi.fn() }));
vi.mock("./features/agents/live", () => ({ initRunTranscripts: vi.fn(), installAgentLiveLifecycle: vi.fn() }));
vi.mock("./features/agents/memories", () => ({ initMemoryTimelines: vi.fn(), installMemoryLifecycle: vi.fn() }));
vi.mock("./features/agents/model-picker", () => ({ initModelPickers: vi.fn(), installModelPickerLifecycle: vi.fn() }));
vi.mock("./features/agents/navigation", () => ({ installAgentNavigation: vi.fn() }));
vi.mock("./features/providers", () => ({ initProviders: vi.fn() }));
vi.mock("./shared/clipboard", () => ({ installCopyButtons: vi.fn() }));
vi.mock("./shared/clickable-rows", () => ({ initClickableRows: vi.fn() }));
vi.mock("./shared/presentation", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./shared/presentation")>()),
  startRunningDurationTicker: vi.fn(),
}));

import "./app";

afterEach(() => {
  document.body.replaceChildren();
});

describe("HTMX initialization", () => {
  it("animates live balance changes before reseeding, keeping digits visible across rapid swaps", () => {
    const animate = vi.fn();
    const originalAnimate = HTMLElement.prototype.animate;
    HTMLElement.prototype.animate = animate;
    try {
      const balance = document.createElement("div");
      balance.setAttribute("sse-swap", "balance");
      document.body.append(balance);
      const swap = (value: string) => {
        balance.innerHTML = `<div class="number-roll" data-animate-key="test-live-pnl" data-raw-value="${value}">${Array.from(value, (digit) => `<span class="number-digit" data-digit="${digit}">${digit}</span>`).join("")}</div>`;
        balance.dispatchEvent(new CustomEvent("htmx:afterSwap", { bubbles: true, detail: { elt: balance, target: balance } }));
      };

      swap("10");
      expect(animate).not.toHaveBeenCalled();
      swap("11");
      expect(animate).toHaveBeenCalledTimes(2);
      expect(animate.mock.calls[0][0][0].color).toBe("#4ade80");
      swap("9");
      expect(animate).toHaveBeenCalledTimes(3);
      expect(animate.mock.calls[2][0][0].color).toBe("#f87171");
      for (const [keyframes] of animate.mock.calls) {
        expect(keyframes.every((frame: Keyframe) => frame.opacity === undefined || frame.opacity === "1")).toBe(true);
      }
      swap("9");
      expect(animate).toHaveBeenCalledTimes(3);
    } finally {
      HTMLElement.prototype.animate = originalAnimate;
    }
  });

  it("formats timestamps in an outerHTML replacement", () => {
    const timestamp = "2026-06-27T00:01:00Z";
    const replacement = document.createElement("section");
    replacement.innerHTML = `<time class="local-datetime" data-local-format="datetime" datetime="${timestamp}">2026-06-27 00:01 UTC</time>`;
    document.body.append(replacement);

    document.dispatchEvent(
      new CustomEvent("htmx:afterSwap", {
        detail: { elt: replacement, target: document.createElement("section") },
      }),
    );

    expect(replacement.querySelector("time")?.textContent).toBe(
      new Intl.DateTimeFormat(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      }).format(new Date(timestamp)),
    );
    expect(replacement.querySelector("time")?.classList.contains("local-datetime-ready")).toBe(true);
  });
});
