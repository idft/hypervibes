const FILE_PANE_WIDTH_KEY = "workspace-file-pane-width";
const MIN_FILE_PANE_WIDTH = 220;
const MIN_PREVIEW_PANE_WIDTH = 320;
const MAX_FILE_PANE_WIDTH = 640;

export function initWorkspaceExplorers(root: ParentNode = document) {
  const explorers: HTMLElement[] = root instanceof HTMLElement && root.matches("[data-workspace-explorer]")
    ? [root]
    : Array.from(root.querySelectorAll<HTMLElement>("[data-workspace-explorer]"));
  explorers.forEach(initializeExplorer);
}

function initializeExplorer(explorer: HTMLElement) {
  if (explorer.dataset.workspaceInitialized === "true") return;
  explorer.dataset.workspaceInitialized = "true";
  initializeTree(explorer);
  initializeResizer(explorer);
}

function initializeTree(explorer: HTMLElement) {
  const tree = explorer.querySelector<HTMLElement>("[data-workspace-tree]");
  if (!tree) return;
  const entries = Array.from(tree.querySelectorAll<HTMLElement>("[data-workspace-entry]"));
  const collapsed = new Set(
    entries.filter((entry) => entry.hasAttribute("data-workspace-folder")).map((entry) => entry.dataset.workspacePath ?? ""),
  );
  const selected = entries.find((entry) => entry.hasAttribute("data-workspace-selected"));
  if (selected?.dataset.workspacePath) {
    for (const folder of ancestors(selected.dataset.workspacePath)) collapsed.delete(folder);
  }

  const render = () => {
    entries.forEach((entry) => {
      const path = entry.dataset.workspacePath;
      if (!path) return;
      const hidden = ancestors(path).some((folder) => collapsed.has(folder));
      entry.hidden = hidden;
      entry.classList.toggle("hidden", hidden);
      if (!entry.hasAttribute("data-workspace-folder")) return;
      const expanded = !collapsed.has(path);
      entry.setAttribute("aria-expanded", String(expanded));
      entry.querySelector("[data-workspace-chevron]")?.classList.toggle("rotate-90", expanded);
    });
  };
  tree.addEventListener("click", (event) => {
    const folder = (event.target as Element).closest<HTMLElement>("[data-workspace-folder]");
    const path = folder?.dataset.workspacePath;
    if (!folder || !path) return;
    if (collapsed.has(path)) collapsed.delete(path); else collapsed.add(path);
    render();
  });
  render();
}

function initializeResizer(explorer: HTMLElement) {
  const pane = explorer.querySelector<HTMLElement>("[data-workspace-file-pane]");
  const resizer = explorer.querySelector<HTMLElement>("[data-workspace-resizer]");
  if (!pane || !resizer) return;
  const filePane = pane;
  const resizeHandle = resizer;
  const storedWidth = Number(window.sessionStorage.getItem(FILE_PANE_WIDTH_KEY));
  if (Number.isFinite(storedWidth)) setWidth(storedWidth);

  resizeHandle.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    resizeHandle.setPointerCapture(event.pointerId);
    explorer.dataset.workspaceResizing = "true";
  });
  resizeHandle.addEventListener("pointermove", (event) => {
    if (!resizeHandle.hasPointerCapture(event.pointerId)) return;
    setWidth(event.clientX - explorer.getBoundingClientRect().left);
  });
  resizeHandle.addEventListener("pointerup", (event) => finishResize(event.pointerId));
  resizeHandle.addEventListener("pointercancel", (event) => finishResize(event.pointerId));
  resizeHandle.addEventListener("keydown", (event) => {
    const current = filePane.getBoundingClientRect().width;
    if (event.key === "ArrowLeft") setWidth(current - 16);
    else if (event.key === "ArrowRight") setWidth(current + 16);
    else return;
    event.preventDefault();
  });

  function finishResize(pointerId: number) {
    if (!resizeHandle.hasPointerCapture(pointerId)) return;
    resizeHandle.releasePointerCapture(pointerId);
    delete explorer.dataset.workspaceResizing;
    window.sessionStorage.setItem(FILE_PANE_WIDTH_KEY, String(filePane.getBoundingClientRect().width));
  }

  function setWidth(width: number) {
    const maximum = Math.min(MAX_FILE_PANE_WIDTH, explorer.clientWidth - MIN_PREVIEW_PANE_WIDTH);
    explorer.style.setProperty("--workspace-file-pane-width", `${Math.max(MIN_FILE_PANE_WIDTH, maximum > MIN_FILE_PANE_WIDTH ? Math.min(width, maximum) : MIN_FILE_PANE_WIDTH)}px`);
  }
}

function ancestors(path: string) {
  const components = path.split("/");
  components.pop();
  const paths: string[] = [];
  for (let index = 1; index <= components.length; index += 1) paths.push(components.slice(0, index).join("/"));
  return paths;
}
