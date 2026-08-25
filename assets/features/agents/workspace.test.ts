import { afterEach, describe, expect, it } from "vitest";

import { initWorkspaceExplorers } from "./workspace";

afterEach(() => {
  document.body.replaceChildren();
  window.sessionStorage.clear();
});

describe("workspace explorer", () => {
  it("starts folders collapsed and expands selected file ancestors", () => {
    document.body.innerHTML = `
      <section data-workspace-explorer>
        <aside data-workspace-file-pane></aside>
        <div data-workspace-resizer></div>
        <div data-workspace-tree>
          <button data-workspace-entry data-workspace-folder data-workspace-path=".opencode" data-workspace-depth="0"><svg data-workspace-chevron></svg></button>
          <button data-workspace-entry data-workspace-folder data-workspace-path=".opencode/agents" data-workspace-depth="1"><svg data-workspace-chevron></svg></button>
          <a data-workspace-entry data-workspace-path=".opencode/agents/agent-conversations.md" data-workspace-depth="2"></a>
          <a data-workspace-entry data-workspace-path="README.md" data-workspace-depth="0"></a>
        </div>
      </section>
    `;
    initWorkspaceExplorers();

    const rootFolder = document.querySelector<HTMLElement>('[data-workspace-path=".opencode"]');
    const nestedFolder = document.querySelector<HTMLElement>('[data-workspace-path=".opencode/agents"]');
    const nestedFile = document.querySelector<HTMLElement>('[data-workspace-path=".opencode/agents/agent-conversations.md"]');
    if (!rootFolder || !nestedFolder || !nestedFile) throw new Error("workspace tree was not rendered");

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
      <section data-workspace-explorer>
        <aside data-workspace-file-pane></aside>
        <div data-workspace-resizer></div>
        <div data-workspace-tree>
          <button data-workspace-entry data-workspace-folder data-workspace-path="scripts" data-workspace-depth="0"><svg data-workspace-chevron></svg></button>
          <a data-workspace-entry data-workspace-path="scripts/strategy.rs" data-workspace-depth="1" data-workspace-selected></a>
        </div>
      </section>
    `;
    initWorkspaceExplorers();

    const folder = document.querySelector<HTMLElement>('[data-workspace-path="scripts"]');
    const file = document.querySelector<HTMLElement>('[data-workspace-path="scripts/strategy.rs"]');
    if (!folder || !file) throw new Error("workspace tree was not rendered");

    expect(folder.getAttribute("aria-expanded")).toBe("true");
    expect(file.hidden).toBe(false);
  });
});
