// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createTestQueryClient } from "../test/query.js";

vi.mock("../web/indicator-queries.js", () => ({
  indicatorListQueryOptions: () => ({ queryKey: ["catalog"], queryFn: async () => ({ indicators: [{ id: "builtin:ema", name: "EMA", builtIn: true, version: 1 }], nextOffset: null }) }),
  indicatorQueryOptions: () => ({ queryKey: ["detail"], queryFn: async () => ({ descriptor: { name: "EMA", version: 1, inputs: [{ key: "period", title: "Period", type: "int", default: 20, min: 2, max: 500 }] } }) }),
}));
import { ChartIndicatorControls } from "./chart-indicator-controls.js";

let root: Root, container: HTMLDivElement;
const change = vi.fn();
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((element) => element.textContent === text)!;
async function click(text: string) {
  await act(async () => { button(text).click(); await new Promise((resolve) => setTimeout(resolve, 25)); });
}
beforeEach(async () => {
  change.mockReset().mockResolvedValue(true);
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  await act(async () => root.render(<QueryClientProvider client={createTestQueryClient()}><ChartIndicatorControls agentId="agent" instances={[]} disabled={false} onChange={change} /></QueryClientProvider>));
  await click("Indicators (0)");
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

it("removes library link, explanatory text, and built-in versions", () => {
  expect(document.body.textContent).not.toContain("Open indicator library");
  expect(document.body.textContent).not.toContain("Latest versions");
  expect(document.body.textContent).toContain("EMA · Built-in");
  expect(document.body.textContent).not.toContain("v1");
});

it("configures inputs before adding the indicator", async () => {
  await click("Add");
  expect(change).not.toHaveBeenCalled();
  expect(document.querySelectorAll("dialog")).toHaveLength(1);
  const input = document.querySelector<HTMLInputElement>('input[type="number"]')!;
  expect(input.value).toBe("20");
  const advanced = document.querySelector("details")!;
  expect(advanced.open).toBe(false);
  expect(advanced.querySelector("summary")?.textContent).toBe("Advanced options");
  expect(advanced.querySelector("select")).not.toBeNull();
  expect(advanced.querySelector("textarea")).not.toBeNull();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "30");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await click("Add to chart");
  expect(change).toHaveBeenCalledWith([expect.objectContaining({ indicatorId: "builtin:ema", inputs: { period: 30 }, configuredVersion: 1 })]);
  expect(document.querySelector("dialog")).toBeNull();
});

it("shows chart indicators by name in a table with icon actions", async () => {
  const instance = { id: "instance", indicatorId: "builtin:ema", configuredVersion: 1, inputs: {}, pane: "auto" as const, hidden: false, styles: {} };
  await act(async () => {
    root.render(<QueryClientProvider client={createTestQueryClient()}><ChartIndicatorControls agentId="agent" instances={[instance]} disabled={false} onChange={change} /></QueryClientProvider>);
    await new Promise((resolve) => setTimeout(resolve, 25));
  });
  const table = document.querySelector('table[aria-label="Indicators on chart"]')!;
  expect(table.querySelector("tbody td")?.textContent).toBe("EMA");
  expect(table.textContent).not.toContain("builtin:");
  const action = (label: string) => table.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
  expect(action("Hide EMA").querySelector("svg")).not.toBeNull();
  expect(action("Configure EMA").querySelector("svg")).not.toBeNull();
  expect(action("Move EMA up").disabled).toBe(true);
  await act(async () => action("Hide EMA").click());
  expect(change).toHaveBeenLastCalledWith([{ ...instance, hidden: true }]);
  await act(async () => action("Remove EMA").click());
  expect(change).toHaveBeenLastCalledWith([]);
  await act(async () => { action("Configure EMA").click(); await new Promise((resolve) => setTimeout(resolve, 25)); });
  expect(document.body.textContent).toContain("Configure indicator");
});

it("keeps the configuration open when adding fails", async () => {
  change.mockResolvedValue(false);
  await click("Add");
  await click("Add to chart");
  expect(document.body.textContent).toContain("Add indicator");
  expect(document.querySelectorAll("dialog")).toHaveLength(1);
});

it("cancels adding without changing the chart", async () => {
  await click("Add");
  await click("Cancel");
  expect(change).not.toHaveBeenCalled();
  expect(document.body.textContent).toContain("Chart indicators");
});
