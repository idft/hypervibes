// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "@tanstack/react-router";
import { expect, it, vi } from "vitest";

vi.mock("../web/agents.js", async () => import("./screenshot-boundaries.js"));
vi.mock("../web/subagents.js", async () => import("./screenshot-boundaries.js"));
vi.mock("../web/indicators.js", async () => import("./screenshot-boundaries.js"));
vi.mock("../web/auth-client.js", async () => import("./screenshot-boundaries.js"));
vi.mock("../components/agent-events.js", async () => import("./screenshot-events.js"));

import { createDashboardScreenshotRouter } from "./dashboard-screenshot.js";
import { screenshotAddress } from "./dashboard-fixture.js";
import { sendAgentMessage, authClient } from "./screenshot-boundaries.js";

it("renders the native dashboard, identity, summaries, and empty chat without a network connection", async () => {
  const fetch = vi.fn(() => { throw new Error("Unexpected request"); });
  const eventSource = vi.fn(() => { throw new Error("Unexpected live stream"); });
  vi.stubGlobal("fetch", fetch);
  vi.stubGlobal("EventSource", eventSource);
  vi.stubGlobal("scrollTo", vi.fn());
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  const router = createDashboardScreenshotRouter("109440");
  try {
    await act(async () => { await router.load(); root.render(<RouterProvider router={router} />); });
    await act(async () => {});
    expect(document.documentElement.dataset.screenshotReady).toBe("true");
    expect(container.querySelector("[inert]")).not.toBeNull();
    expect(container.querySelector('[data-agent-selector] summary')?.textContent).toContain("Atlas Momentum");
    expect(container.querySelector('a[href="/account"]')?.getAttribute("title")).toBe(screenshotAddress);
    expect(container.querySelector('[aria-label="Account overview"]')?.textContent).toContain("1,472.53");
    expect([...container.querySelectorAll("h3")].map((heading) => heading.textContent)).toEqual(["Positions", "Open orders", "Sub-agents", "Chat"]);
    expect(container.querySelectorAll("table tbody tr")).toHaveLength(3);
    expect(container.querySelectorAll("h4")).toHaveLength(3);
    expect(container.textContent).toContain("Daily Reviewer");
    expect(container.querySelectorAll('svg[aria-label$="realized account balance"]')).toHaveLength(2);
    expect(container.querySelector('[aria-label="Sub-agents"]')?.textContent).toContain("Risk controls staged for the current BTC position.");
    expect(container.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("");
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(fetch).not.toHaveBeenCalled();
    expect(eventSource).not.toHaveBeenCalled();
    expect(container.querySelector('nav a[aria-label="Dashboard"]')?.getAttribute("aria-current")).toBe("page");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    vi.unstubAllGlobals();
  }
  expect(document.documentElement.dataset.screenshotReady).toBeUndefined();
});

it("rejects application operations locally rather than sending them to a server", () => {
  expect(() => sendAgentMessage()).toThrow("Application operations are unavailable");
  expect(() => authClient.signOut()).toThrow("Application operations are unavailable");
});
