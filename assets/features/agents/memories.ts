function selectedMemoryRoots(root: ParentNode) {
  return root instanceof HTMLElement && root.matches("[data-agent-memories]") ? [root] : Array.from(root.querySelectorAll<HTMLElement>("[data-agent-memories]"));
}

function setActive(item: HTMLElement) {
  const root = item.closest<HTMLElement>("[data-agent-memories]");
  if (!root) return;
  root.dataset.selectedMemoryId = item.dataset.memoryId ?? "";
  root.querySelectorAll<HTMLElement>("[data-memory-timeline-item]").forEach((candidate) => candidate.setAttribute("aria-pressed", candidate === item ? "true" : "false"));
}

export function restoreSelectedMemoryItems(root: ParentNode = document) {
  selectedMemoryRoots(root).forEach((memoryRoot) => {
    const selectedId = memoryRoot.querySelector<HTMLElement>("#memory-detail[data-memory-detail-id]")?.dataset.memoryDetailId ?? memoryRoot.dataset.selectedMemoryId;
    if (!selectedId) return;
    memoryRoot.dataset.selectedMemoryId = selectedId;
    memoryRoot.querySelectorAll<HTMLElement>("[data-memory-timeline-item]").forEach((item) => item.setAttribute("aria-pressed", item.dataset.memoryId === selectedId ? "true" : "false"));
  });
}

export function initMemoryTimelines(root: ParentNode = document) {
  selectedMemoryRoots(root).forEach((memoryRoot) => {
    if (!memoryRoot.dataset.selectedMemoryId) {
      const selected = memoryRoot.querySelector<HTMLElement>('[data-memory-timeline-item][aria-pressed="true"]');
      if (selected?.dataset.memoryId) memoryRoot.dataset.selectedMemoryId = selected.dataset.memoryId;
    }
  });
  root.querySelectorAll<HTMLElement>(".memory-timeline-scroll").forEach((container) => {
    if (container.dataset.dragScrollBound === "true") return;
    container.dataset.dragScrollBound = "true";
    let startX = 0; let startScrollLeft = 0; let dragging = false;
    container.addEventListener("pointerdown", (event) => { if (event.button !== 0) return; startX = event.clientX; startScrollLeft = container.scrollLeft; dragging = false; container.setPointerCapture(event.pointerId); });
    container.addEventListener("pointermove", (event) => { if (!container.hasPointerCapture(event.pointerId)) return; const delta = event.clientX - startX; if (Math.abs(delta) > 6) { dragging = true; container.dataset.dragging = "true"; container.scrollLeft = startScrollLeft - delta; } });
    container.addEventListener("pointerup", (event) => { if (container.hasPointerCapture(event.pointerId)) container.releasePointerCapture(event.pointerId); window.setTimeout(() => { dragging = false; container.dataset.dragging = "false"; }, 0); });
    container.addEventListener("click", (event) => { if (dragging) { event.preventDefault(); event.stopPropagation(); } }, true);
  });
  restoreSelectedMemoryItems(root);
}

export function installMemoryLifecycle() {
  document.addEventListener("htmx:afterRequest", (event) => {
    const element = (event as CustomEvent<{ elt?: Element; successful?: boolean }>).detail.elt;
    if ((event as CustomEvent<{ successful?: boolean }>).detail.successful && element instanceof HTMLElement && element.matches("[data-memory-timeline-item]")) setActive(element);
  });
  document.addEventListener("htmx:afterSwap", (event) => { const target = (event as CustomEvent<{ target?: unknown }>).detail.target; if (!(target instanceof Element)) return; initMemoryTimelines(target); restoreSelectedMemoryItems(target); });
  document.addEventListener("htmx:sseMessage", (event) => { const target = (event as CustomEvent<{ elt?: Element }>).detail.elt; if (target instanceof Element && target.matches('[sse-swap="memories-timeline"]')) restoreSelectedMemoryItems(target.closest("[data-agent-memories]") ?? document); });
}
