import { afterEach, describe, expect, it } from "vitest";

import { installAgentNavigation } from "./navigation";

afterEach(() => {
  document.body.replaceChildren();
  document.documentElement.classList.remove("agent-rail-initially-collapsed");
  window.localStorage.clear();
  window.sessionStorage.clear();
  window.history.replaceState({}, "", "/");
});

describe("agent rail", () => {
  it("defaults to expanded and updates the global navigation preference", () => {
    document.body.innerHTML = `
      <main class="app-content has-agent-rail"></main>
      <nav class="agent-rail" data-agent-rail>
        <button data-agent-rail-toggle aria-expanded="false"></button>
        <svg data-agent-rail-expand-icon></svg>
        <svg data-agent-rail-collapse-icon class="hidden"></svg>
      </nav>
    `;
    window.sessionStorage.setItem("agent-rail-enter-destination", window.location.pathname);
    installAgentNavigation();

    const rail = document.querySelector<HTMLElement>("[data-agent-rail]");
    const content = document.querySelector<HTMLElement>(".app-content");
    const toggle = document.querySelector<HTMLButtonElement>("[data-agent-rail-toggle]");
    if (!rail || !content || !toggle) throw new Error("Agent rail was not rendered");

    expect(rail.classList.contains("agent-rail-expanded")).toBe(true);
    expect(content.classList.contains("agent-rail-expanded")).toBe(true);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(rail.classList.contains("agent-rail-enter")).toBe(false);

    toggle.click();

    expect(rail.classList.contains("agent-rail-expanded")).toBe(false);
    expect(content.classList.contains("agent-rail-expanded")).toBe(false);
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(window.localStorage.getItem("agent-rail-expanded")).toBe("false");

    toggle.click();

    expect(rail.classList.contains("agent-rail-expanded")).toBe(true);
    expect(content.classList.contains("agent-rail-expanded")).toBe(true);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(window.localStorage.getItem("agent-rail-expanded")).toBe("true");
  });

  it("keeps a saved collapsed preference compact during initialization", () => {
    document.body.innerHTML = `
      <main class="app-content has-agent-rail agent-rail-expanded"></main>
      <nav class="agent-rail agent-rail-expanded" data-agent-rail>
        <button data-agent-rail-toggle aria-expanded="true"></button>
        <svg data-agent-rail-expand-icon class="hidden"></svg>
        <svg data-agent-rail-collapse-icon></svg>
      </nav>
    `;
    document.documentElement.classList.add("agent-rail-initially-collapsed");
    window.localStorage.setItem("agent-rail-expanded", "false");
    installAgentNavigation();

    const rail = document.querySelector<HTMLElement>("[data-agent-rail]");
    const content = document.querySelector<HTMLElement>(".app-content");
    if (!rail || !content) throw new Error("Agent rail was not rendered");

    expect(document.documentElement.classList.contains("agent-rail-initially-collapsed")).toBe(false);
    expect(rail.classList.contains("agent-rail-expanded")).toBe(false);
    expect(content.classList.contains("agent-rail-expanded")).toBe(false);
  });

  it("keeps Chat active for a nested conversation URL", () => {
    window.history.replaceState({}, "", "/agents/test-agent/chat/00000000-0000-0000-0000-000000000000");
    document.body.innerHTML = `
      <nav data-agent-tabs>
        <a data-agent-tab-link href="/agents/test-agent" class="text-zinc-400">Positions</a>
        <a data-agent-tab-link href="/agents/test-agent/chat" aria-current="page" class="bg-zinc-800 text-white">Chat</a>
      </nav>
    `;
    installAgentNavigation();

    const chat = document.querySelector<HTMLAnchorElement>('a[href="/agents/test-agent/chat"]');
    if (!chat) throw new Error("Chat tab was not rendered");

    expect(chat.getAttribute("aria-current")).toBe("page");
    expect(chat.classList.contains("bg-zinc-800")).toBe(true);
    expect(chat.classList.contains("text-white")).toBe(true);

    document.dispatchEvent(new CustomEvent("htmx:afterSwap"));

    expect(chat.getAttribute("aria-current")).toBe("page");
    expect(chat.classList.contains("bg-zinc-800")).toBe(true);
    expect(chat.classList.contains("text-white")).toBe(true);
  });
});
