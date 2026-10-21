// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NotFoundPage, RouteErrorPage } from "./route-status-page.js";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("route status pages", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  });

  it("renders a branded 404 with a home link without needing account or router context", async () => {
    await act(async () => root.render(<NotFoundPage />));
    expect(container.querySelector("h1")?.textContent).toBe("Page not found");
    expect(container.textContent).toContain("404");
    expect(container.textContent).toContain("HyperVibes");
    expect(container.querySelector("img")?.getAttribute("src")).toBe("/static/logo.png");
    const home = Array.from(container.querySelectorAll("a")).find((link) => link.textContent === "Go home");
    expect(home?.getAttribute("href")).toBe("/");
    expect(container.querySelector("button")).toBeNull();
    expect(container.querySelector("section")?.getAttribute("aria-labelledby")).toBe(container.querySelector("h1")?.id);
  });

  it("offers a full page retry and home navigation for errors", async () => {
    const reload = vi.fn();
    const location = window.location;
    vi.spyOn(window, "location", "get").mockReturnValue({ ...location, reload } as Location);
    await act(async () => root.render(<RouteErrorPage />));
    expect(container.querySelector("h1")?.textContent).toBe("Something went wrong");
    const retry = container.querySelector("button")!;
    expect(retry.textContent).toBe("Try again");
    expect(retry.className).toContain("cursor-pointer");
    await act(async () => retry.click());
    expect(reload).toHaveBeenCalledOnce();
    const home = Array.from(container.querySelectorAll("a")).find((link) => link.textContent === "Go home");
    expect(home?.getAttribute("href")).toBe("/");
  });
});
