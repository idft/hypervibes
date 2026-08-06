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
