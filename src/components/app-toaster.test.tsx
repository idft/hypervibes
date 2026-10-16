// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { toast } from "sonner";
import { AppToaster } from "./app-toaster.js";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it("shows dismissible dark success toasts in the top right", async () => {
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })));
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<AppToaster />));
    await act(async () => {
      toast.success("Sub-agent created");
      // Sonner publishes toast updates on the next timer tick.
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    const host = container.querySelector("[data-sonner-toaster]");
    expect(host?.getAttribute("data-x-position")).toBe("right");
    expect(host?.getAttribute("data-y-position")).toBe("top");
    expect(host?.getAttribute("data-sonner-theme")).toBe("dark");
    expect(container.textContent).toContain("Sub-agent created");
    const success = container.querySelector('[data-sonner-toast][data-type="success"]');
    expect(success?.className).toContain("text-emerald-300!");
    expect(success?.querySelector("[data-icon] svg")?.getAttribute("class")).toContain("text-emerald-400");
    const close = container.querySelector<HTMLButtonElement>("[data-close-button]");
    expect(close?.className).toContain("cursor-pointer");
    await act(async () => close!.click());
  } finally {
    await act(async () => { toast.dismiss(); root.unmount(); });
    container.remove(); vi.unstubAllGlobals();
  }
});
