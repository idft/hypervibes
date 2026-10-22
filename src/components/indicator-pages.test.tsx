// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createTestQueryClient } from "../test/query.js";

const mocks = vi.hoisted(() => ({ catalog: vi.fn(), source: vi.fn(), detail: vi.fn(), versions: vi.fn(), preview: vi.fn(), previewSaved: vi.fn(), save: vi.fn(), navigate: vi.fn() }));
vi.mock("../web/indicator-queries.js", () => ({ indicatorKeys: { all: (agentId: string) => ["indicators", agentId] }, indicatorListQueryOptions: (agentId: string, options: unknown) => ({ queryKey: ["indicators", agentId, "list", options], queryFn: () => mocks.catalog(agentId, options) }), indicatorQueryOptions: (agentId: string, indicatorId: string, options: unknown) => ({ queryKey: ["indicators", agentId, "detail", indicatorId, options], queryFn: () => mocks.detail(agentId, indicatorId, options) }), indicatorVersionsQueryOptions: (agentId: string, indicatorId: string, offset: number) => ({ queryKey: ["indicators", agentId, "versions", indicatorId, offset], queryFn: () => mocks.versions(agentId, indicatorId, offset) }) }));
vi.mock("../web/indicators.js", () => ({ getIndicator: mocks.source, previewIndicator: mocks.preview, previewSavedIndicator: mocks.previewSaved, saveIndicator: mocks.save }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, params, search, children, ...rest }: ComponentProps<"a"> & { to: string; params?: { agentId: string; indicatorId?: string }; search?: { copy?: string } }) => <a href={`${to.replace("$agentId", params?.agentId ?? "").replace("$indicatorId", params?.indicatorId ?? "")}${search?.copy ? `?copy=${search.copy}` : ""}`} {...rest}>{children}</a>,
  useNavigate: () => mocks.navigate,
}));
import { AgentIndicatorsPage } from "./agent-indicators-page.js";
import { IndicatorDetailPage } from "./indicator-detail-page.js";

const builtin = { id: "builtin:ema", builtIn: true, name: "EMA", description: "Exponential moving average", version: 1, sourceHash: "a".repeat(64), overlay: true };
const descriptor = { id: "builtin:ema", builtIn: true, name: "EMA", description: "Exponential moving average", interpretation: "Analytical", version: 1, sourceHash: "a".repeat(64), inputs: [{ key: "period", variable: "period", runtimeId: "in_0", title: "Period", type: "int", default: 20, min: 2, max: 500 }], overlay: true };
let container: HTMLDivElement, root: Root, client: QueryClient;
beforeEach(() => {
  vi.clearAllMocks();
  mocks.catalog.mockResolvedValue({ indicators: [builtin], total: 1, nextOffset: null });
  mocks.detail.mockResolvedValue({ descriptor });
  mocks.source.mockResolvedValue({ descriptor, source: { text: '//@version=6\nindicator("EMA")\nplot(close)', offset: 0, total: 40, nextOffset: null } });
  mocks.versions.mockResolvedValue({ versions: [{ version: 1, sourceHash: "a".repeat(64), createdAt: 1, actorKind: "user" }], nextOffset: null });
  mocks.preview.mockResolvedValue({ result: { version: 1, indicators: [], dataset: { count: 3 }, markers: [], drawings: [], values: [], outputs: [], resolvedInputs: {} }, candles: [{ time: 0, open: 1, high: 2, low: 1, close: 2, volume: 1 }] });
  mocks.previewSaved.mockResolvedValue({ result: { version: 1, dataset: { count: 3 }, markers: [], drawings: [], values: [], outputs: [], resolvedInputs: {} }, candles: [] });
  mocks.save.mockResolvedValue({ indicatorId: "custom:12345678-1234-4234-8234-123456789abc", version: 1, sourceHash: "b".repeat(64) });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container); client = createTestQueryClient();
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
const sourceBox = () => [...container.querySelectorAll("label")].find((label) => label.textContent?.startsWith("PineScript source"))!.querySelector("textarea") as HTMLTextAreaElement;
const render = async (element: React.ReactNode) => { await act(async () => { root.render(<QueryClientProvider client={client}>{element}</QueryClientProvider>); await new Promise((resolve) => setTimeout(resolve, 25)); }); };

describe("Agent indicator library page", () => {
  it("lists built-in and custom headers with pagination and links", async () => {
    await render(<AgentIndicatorsPage agentId="agent" search={{ query: "", kind: "all", offset: 0 }} onSearch={() => {}} />);
    expect(container.textContent).toContain("EMA");
    expect(container.textContent).toContain("Built-in · read-only");
    expect(container.querySelector("table")).not.toBeNull();
    expect([...container.querySelectorAll("th")].map((cell) => cell.textContent)).toEqual(["Name", "Kind", "Description"]);
    expect(container.textContent).not.toContain("v1");
    expect(container.textContent).not.toContain("Calculations are on demand");
    expect(container.querySelector("a[href='/agents/agent/indicators/builtin:ema']")).not.toBeNull();
    const descriptionLink = container.querySelector("tbody tr td:nth-child(3) a");
    expect(descriptionLink?.textContent).toBe(builtin.description);
    expect(descriptionLink?.getAttribute("href")).toBe("/agents/agent/indicators/builtin:ema");
    expect(container.querySelector("a[href='/agents/agent/indicators/new']")).not.toBeNull();
    expect(container.querySelector("a[href='/agents/agent/sub-agents/new']")).toBeNull();
    // No source or results leak into a listing.
    expect(container.textContent).not.toContain("//@version");
  });
  it("renders the page while the catalog is still loading", async () => {
    mocks.catalog.mockReturnValue(new Promise(() => {}));
    await render(<AgentIndicatorsPage agentId="agent" search={{ query: "", kind: "all", offset: 0 }} onSearch={() => {}} />);
    expect(container.querySelector("h2")?.textContent).toBe("Indicators");
    expect(container.querySelector("[role=status]")?.textContent).toBe("Loading indicators…");
    expect(container.querySelector("a[href='/agents/agent/indicators/new']")).not.toBeNull();
  });
  it("surfaces catalog failures instead of an empty library", async () => {
    mocks.catalog.mockRejectedValue(new Error("Indicator service unavailable"));
    await render(<AgentIndicatorsPage agentId="agent" search={{ query: "", kind: "all", offset: 0 }} onSearch={() => {}} />);
    expect(container.querySelector("[role=alert]")!.textContent).toContain("unavailable");
  });
});

describe("indicator detail, preview and save", () => {
  it("shows a read-only built-in source with a copy action and previews without saving", async () => {
    await render(<IndicatorDetailPage key="builtin:ema" agentId="agent" indicatorId="builtin:ema" />);
    const breadcrumb = container.querySelector('nav[aria-label="Indicator breadcrumb"]')!;
    expect(breadcrumb.querySelector("a")?.getAttribute("href")).toBe("/agents/agent/indicators");
    expect(breadcrumb.querySelector("h2")?.textContent).toBe("EMA");
    const source = sourceBox();
    expect(source.readOnly).toBe(true);
    expect(source.value).toContain("//@version");
    expect(container.querySelector("a[href='/agents/agent/indicators/new?copy=builtin:ema']")).not.toBeNull();
    await act(async () => { [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Preview")!.click(); await new Promise((resolve) => setTimeout(resolve, 25)); });
    // Unedited saved source previews its stored version; edited source goes through the draft preview.
    expect(mocks.previewSaved).toHaveBeenCalled(); expect(mocks.preview).not.toHaveBeenCalled();
    expect(mocks.save).not.toHaveBeenCalled();
    expect(container.textContent).toContain("closed inputs");
  });
  it("creates a new custom indicator through validated save and navigates to it", async () => {
    await render(<IndicatorDetailPage key="new" agentId="agent" indicatorId="new" />);
    expect(container.querySelector('nav[aria-label="Indicator breadcrumb"] h2')?.textContent).toBe("New indicator");
    const source = sourceBox();
    await act(async () => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(source, '//@version=6\nindicator("Mine")\nplot(close)'); source.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => { [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Save immediately")!.click(); await new Promise((resolve) => setTimeout(resolve, 25)); });
    expect(mocks.save).toHaveBeenCalledWith({ data: { agentId: "agent", operationKey: expect.any(String), request: expect.objectContaining({ source: '//@version=6\nindicator("Mine")\nplot(close)', validation: expect.objectContaining({ coin: "BTC", interval: "1h" }) }) } });
    expect(mocks.navigate).toHaveBeenCalled();
    expect(container.textContent).toContain("Saved");
  });
  it("keeps a built-in immutable and reports source-version history", async () => {
    await render(<IndicatorDetailPage key="builtin:ema" agentId="agent" indicatorId="builtin:ema" />);
    expect(sourceBox().readOnly).toBe(true);
    expect(container.textContent).toContain("read-only");
    expect(container.textContent).toContain("Source history");
    expect(mocks.versions).toHaveBeenCalled();
  });
});
