// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { configurationToolNames } from "../agents/configuration-contracts.js";
import { ToolPicker } from "./subagent-config.js";

describe("Agent configuration tool selection", () => {
  let root: Root, container: HTMLDivElement;
  const save = vi.fn().mockResolvedValue(undefined);
  const click = async (label: string) => act(async () => {
    [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === label || button.getAttribute("aria-label") === label)!.click();
  });
  const checkbox = (name: string) => [...document.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find((input) => input.labels?.[0]?.textContent === name);
  beforeEach(() => {
    save.mockClear();
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

  it("always includes the root configuration tools and prevents individual or group removal", async () => {
    await act(async () => root.render(<ToolPicker tools={[]} main onChange={save} />));
    expect(container.textContent).toContain(configurationToolNames.join(", "));
    await click("Edit tools");
    for (const name of configurationToolNames) {
      expect(checkbox(name)?.checked).toBe(true);
      expect(checkbox(name)?.disabled).toBe(true);
    }
    const none = document.querySelector<HTMLButtonElement>('button[aria-label="No tools in Agent configuration · Orchestrator only"]')!;
    expect(none.disabled).toBe(true);
    await act(async () => none.click());
    await click("Apply");
    expect(save).toHaveBeenCalledExactlyOnceWith([...configurationToolNames]);
  });

  it.each([false, true])("exposes selectable documentation tools for main=%s", async (main) => {
    await act(async () => root.render(<ToolPicker tools={[]} main={main} onChange={save} />));
    await click("Edit tools");
    if (!main) {
      expect(document.body.textContent).not.toContain("Agent configuration · Orchestrator only");
      for (const name of configurationToolNames) expect(checkbox(name)).toBeUndefined();
    }
    for (const name of ["list_docs", "read_doc", "search_docs"]) {
      expect(checkbox(name)?.checked).toBe(false);
      expect(checkbox(name)?.disabled).toBe(false);
    }
    await click("All tools in HyperVibes · Documentation");
    for (const name of ["list_docs", "read_doc", "search_docs"]) expect(checkbox(name)?.checked).toBe(true);
    await click("No tools in HyperVibes · Documentation");
    for (const name of ["list_docs", "read_doc", "search_docs"]) expect(checkbox(name)?.checked).toBe(false);
    await act(async () => checkbox("read_doc")!.click());
    await click("Apply");
    expect(save).toHaveBeenCalledExactlyOnceWith(main ? [...configurationToolNames, "read_doc"] : ["read_doc"]);
  });
});
