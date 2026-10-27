// @vitest-environment jsdom
import { act, type ComponentProps, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createTestQueryClient } from "../test/query.js";
import type { NotificationRecord } from "../notifications/contracts.js";

const mocks = vi.hoisted(() => ({ list: vi.fn(), get: vi.fn(), settings: vi.fn(), save: vi.fn(), event: vi.fn(), success: vi.fn() }));
vi.mock("../web/notifications.js", () => ({ listNotifications: mocks.list, getNotification: mocks.get, getNotificationSettings: mocks.settings, saveNotificationSettings: mocks.save }));
vi.mock("./agent-events.js", () => ({ useAgentEvent: mocks.event, useAgentEventsStatus: () => ({ interrupted: false, streamErrors: {} }) }));
vi.mock("sonner", () => ({ toast: { success: mocks.success } }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, params, search, hash, children, ...rest }: ComponentProps<"a"> & { to: string; params?: Record<string, string>; search?: { page?: number }; hash?: string }) => {
    let href = to; for (const [key, value] of Object.entries(params ?? {})) href = href.replace(`$${key}`, value);
    return <a href={href + (search?.page ? `?page=${search.page}` : "") + (hash ? `#${hash}` : "")} {...rest}>{children}</a>;
  },
}));
import { AgentNotificationDetailPage, AgentNotificationsPage } from "./agent-notifications-page.js";
import { NotificationSettingsSection } from "./notification-settings-section.js";
const row: NotificationRecord = { id: "n1", agentId: "agent", category: "message", title: "Daily review", body: "A useful summary\nFull review: m1", severity: "info", source: "Reviewer", subagentId: "reviewer", runId: "run1", conversationId: 3, orderRequestId: null, createdAt: 1000, delivery: "sent", attempts: 0, sentAt: 2000, error: null };
let container: HTMLDivElement, root: Root, client: QueryClient;
beforeEach(() => {
  vi.resetAllMocks(); client = createTestQueryClient();
  mocks.list.mockResolvedValue({ items: [row], total: 1, page: 1, pages: 1 }); mocks.get.mockResolvedValue(row);
  mocks.settings.mockResolvedValue({ enabled: true, submissions: false, fills: true });
  mocks.save.mockImplementation(async ({ data }: { data: { enabled: boolean; submissions: boolean; fills: boolean } }) => ({ enabled: data.enabled, submissions: data.submissions, fills: data.fills }));
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); client.clear(); container.remove(); });
async function render(element: ReactNode) { await act(async () => { root.render(<QueryClientProvider client={client}>{element}</QueryClientProvider>); await new Promise((resolve) => setTimeout(resolve, 20)); }); }

describe("Agent notification pages", () => {
  it("matches the existing page shell and links details/settings without unread counters", async () => {
    await render(<AgentNotificationsPage agentId="agent" page={1} onPage={() => {}} />);
    expect(container.querySelector("section")?.className).toContain("rounded-2xl border border-zinc-800");
    expect(container.querySelector("h2")?.textContent).toBe("Notifications");
    expect(container.querySelector('a[href="/agents/agent/notifications/n1"]')).not.toBeNull();
    expect(container.querySelector('a[href="/agents/agent/settings#notifications"]')).not.toBeNull();
    expect(container.textContent).toContain("Sent to Telegram");
    expect(container.textContent?.toLowerCase()).not.toMatch(/unread|mark read|mark unread/);
    expect(mocks.event.mock.calls[0]?.[0]).toBe("notifications");
  });
  it("uses the detail breadcrumb pattern and provides a source run link", async () => {
    await render(<AgentNotificationDetailPage agentId="agent" notificationId="n1" />);
    const breadcrumb = container.querySelector('nav[aria-label="Notification breadcrumb"]')!;
    expect(breadcrumb.querySelector("a")?.textContent).toBe("← Notifications");
    expect(breadcrumb.querySelector("a")?.getAttribute("href")).toBe("/agents/agent/notifications?page=1");
    expect(breadcrumb.querySelector('h2[aria-current="page"]')?.textContent).toBe("Daily review");
    expect(container.textContent).toContain(row.body);
    expect(container.querySelector('a[href="/agents/agent/sub-agents/reviewer/runs/run1"]')).not.toBeNull();
  });
  it("renders loading, empty and error states without claiming errors are empty history", async () => {
    mocks.list.mockReturnValue(new Promise(() => {}));
    await render(<AgentNotificationsPage agentId="agent" page={1} onPage={() => {}} />);
    expect(container.querySelector('[role="status"]')?.textContent).toBe("Loading notifications…");
    client.clear(); mocks.list.mockResolvedValue({ items: [], total: 0, pages: 1, page: 1 });
    await render(<AgentNotificationsPage key="empty" agentId="agent" page={1} onPage={() => {}} />);
    expect(container.textContent).toContain("No notifications yet");
    client.clear(); mocks.list.mockRejectedValue(new Error("History unavailable"));
    await render(<AgentNotificationsPage key="error" agentId="agent" page={1} onPage={() => {}} />);
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("History unavailable");
    expect(container.textContent).not.toContain("No notifications yet");
  });
  it("paginates and refreshes active queries on a live notification event", async () => {
    mocks.list.mockResolvedValue({ items: [row], total: 51, page: 1, pages: 2 });
    const onPage = vi.fn(); await render(<AgentNotificationsPage agentId="agent" page={1} onPage={onPage} />);
    await act(async () => [...container.querySelectorAll("button")].find((b) => b.textContent === "Next")!.click());
    expect(onPage).toHaveBeenCalledWith(2);
    const handler = mocks.event.mock.calls.at(-1)![1] as () => void;
    await act(async () => { handler(); }); expect(mocks.list).toHaveBeenCalledTimes(2);
  });
});

describe("Notification settings", () => {
  it("retains child choices when master is off and saves all three booleans", async () => {
    await render(<NotificationSettingsSection agentId="agent" />);
    expect(container.querySelector("h3")?.textContent).toBe("Notifications");
    const inputs = [...container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];
    expect(inputs[0]!.labels?.[0]?.textContent).toBe("Enable Telegram notifications");
    expect(inputs.map((element) => element.checked)).toEqual([true, false, true]);
    await act(async () => inputs[0]!.click());
    expect(inputs.map((element) => element.checked)).toEqual([false, false, true]);
    expect(inputs[1]!.closest("fieldset")?.disabled).toBe(true);
    expect(container.querySelector('button[type="submit"]')?.textContent).toBe("Save Telegram notification settings");
    await act(async () => container.querySelector<HTMLButtonElement>('button[type="submit"]')!.click());
    expect(mocks.save).toHaveBeenCalledWith({ data: { agentId: "agent", enabled: false, submissions: false, fills: true } });
    expect(mocks.success).toHaveBeenCalledWith("Notification settings saved");
  });
  it("shows save failures and lets the operator retry", async () => {
    mocks.save.mockRejectedValue(new Error("Unable to save"));
    await render(<NotificationSettingsSection agentId="agent" />);
    await act(async () => container.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
    await act(async () => container.querySelector<HTMLButtonElement>('button[type="submit"]')!.click());
    expect(container.querySelector('[role="alert"]')?.textContent).toBe("Unable to save");
    expect(container.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(false);
  });
});
