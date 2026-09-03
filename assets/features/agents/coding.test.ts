import { afterEach, describe, expect, it } from "vitest";

import { initCodingExplorers } from "./coding";

afterEach(() => {
  document.body.replaceChildren();
  window.sessionStorage.clear();
});

describe("coding explorer", () => {
  it("starts folders collapsed and expands selected file ancestors", () => {
    document.body.innerHTML = `
      <section data-coding-explorer>
        <aside data-coding-file-pane></aside>
        <div data-coding-resizer></div>
        <div data-coding-tree>
          <button data-coding-entry data-coding-folder data-coding-path=".opencode" data-coding-depth="0"><svg data-coding-chevron></svg></button>
          <button data-coding-entry data-coding-folder data-coding-path=".opencode/agents" data-coding-depth="1"><svg data-coding-chevron></svg></button>
          <a data-coding-entry data-coding-path=".opencode/agents/agent-conversations.md" data-coding-depth="2"></a>
          <a data-coding-entry data-coding-path="README.md" data-coding-depth="0"></a>
        </div>
      </section>
    `;
    initCodingExplorers();

    const rootFolder = document.querySelector<HTMLElement>('[data-coding-path=".opencode"]');
    const nestedFolder = document.querySelector<HTMLElement>('[data-coding-path=".opencode/agents"]');
    const nestedFile = document.querySelector<HTMLElement>('[data-coding-path=".opencode/agents/agent-conversations.md"]');
    if (!rootFolder || !nestedFolder || !nestedFile) throw new Error("coding tree was not rendered");

    expect(rootFolder.getAttribute("aria-expanded")).toBe("false");
    expect(nestedFolder.hidden).toBe(true);
    expect(nestedFolder.classList.contains("hidden")).toBe(true);
    expect(nestedFile.hidden).toBe(true);

    rootFolder.click();
    expect(rootFolder.getAttribute("aria-expanded")).toBe("true");
    expect(nestedFolder.hidden).toBe(false);
    expect(nestedFolder.classList.contains("hidden")).toBe(false);
    expect(nestedFile.hidden).toBe(true);
  });

  it("expands the selected file's ancestors after an HTMX swap", () => {
    document.body.innerHTML = `
      <section data-coding-explorer>
        <aside data-coding-file-pane></aside>
        <div data-coding-resizer></div>
        <div data-coding-tree>
          <button data-coding-entry data-coding-folder data-coding-path="scripts" data-coding-depth="0"><svg data-coding-chevron></svg></button>
          <a data-coding-entry data-coding-path="scripts/strategy.rs" data-coding-depth="1" data-coding-selected></a>
        </div>
      </section>
    `;
    initCodingExplorers();

    const folder = document.querySelector<HTMLElement>('[data-coding-path="scripts"]');
    const file = document.querySelector<HTMLElement>('[data-coding-path="scripts/strategy.rs"]');
    if (!folder || !file) throw new Error("coding tree was not rendered");

    expect(folder.getAttribute("aria-expanded")).toBe("true");
    expect(file.hidden).toBe(false);
  });
});
