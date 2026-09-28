import { renderLocalDateTimes } from "../../shared/presentation";

function localTodayDate() {
  const today = new Date();
  const month = String(today.getMonth() + 1).padStart(2, "0");
  const day = String(today.getDate()).padStart(2, "0");
  return `${today.getFullYear()}-${month}-${day}`;
}

function updateDateLimits(form: HTMLFormElement) {
  const today = localTodayDate();
  form.querySelectorAll<HTMLInputElement>('input[type="date"]').forEach((input) => { input.max = today; });
}

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
    const timezone = memoryRoot.querySelector<HTMLInputElement>("[data-memory-timezone]");
    const browserTimezone = () => Intl.DateTimeFormat().resolvedOptions().timeZone ?? "UTC";
    if (timezone && !timezone.value) timezone.value = browserTimezone();
    const form = memoryRoot.querySelector<HTMLFormElement>("[data-memory-date-form]");
    if (form && form.dataset.bound !== "true") {
      form.dataset.bound = "true";
      form.addEventListener("submit", () => { updateDateLimits(form); if (timezone) timezone.value = browserTimezone(); });
    }
    if (form) updateDateLimits(form);
    const customToggle = memoryRoot.querySelector<HTMLButtonElement>("[data-memory-custom-toggle]");
    if (customToggle && form && customToggle.dataset.bound !== "true") {
      customToggle.dataset.bound = "true";
      customToggle.addEventListener("click", () => {
        updateDateLimits(form);
        form.classList.remove("hidden");
        customToggle.setAttribute("aria-expanded", "true");
        form.querySelector<HTMLInputElement>('input[name="start"]')?.focus();
      });
    }
    const cancel = form?.querySelector<HTMLButtonElement>("[data-memory-custom-cancel]");
    if (cancel && customToggle && form && cancel.dataset.bound !== "true") {
      cancel.dataset.bound = "true";
      cancel.addEventListener("click", () => {
        form.reset();
        form.classList.add("hidden");
        customToggle.setAttribute("aria-expanded", "false");
        customToggle.focus();
      });
    }
    form?.querySelectorAll<HTMLInputElement>('input[type="date"]').forEach((input) => {
      if (input.dataset.pickerBound === "true") return;
      input.dataset.pickerBound = "true";
      input.addEventListener("focus", () => { if (form) updateDateLimits(form); });
      input.addEventListener("click", () => {
        if (form) updateDateLimits(form);
        try { input.showPicker?.(); } catch { input.focus(); }
      });
    });
    if (!memoryRoot.dataset.selectedMemoryId) {
      const selected = memoryRoot.querySelector<HTMLElement>('[data-memory-timeline-item][aria-pressed="true"]');
      if (selected?.dataset.memoryId) memoryRoot.dataset.selectedMemoryId = selected.dataset.memoryId;
    }
  });
  restoreSelectedMemoryItems(root);
}

export function installMemoryLifecycle() {
  document.addEventListener("htmx:afterRequest", (event) => {
    const element = (event as CustomEvent<{ elt?: Element; successful?: boolean }>).detail.elt;
    if ((event as CustomEvent<{ successful?: boolean }>).detail.successful && element instanceof HTMLElement && element.matches("[data-memory-timeline-item]")) setActive(element);
  });
  document.addEventListener("htmx:afterSwap", (event) => { const target = (event as CustomEvent<{ target?: unknown }>).detail.target; if (!(target instanceof Element)) return; initMemoryTimelines(target); restoreSelectedMemoryItems(target); });
  document.addEventListener("htmx:sseMessage", (event) => { const target = (event as CustomEvent<{ elt?: Element }>).detail.elt; if (target instanceof Element && target.matches('[sse-swap="memories-browser"]')) { const root = target.closest("[data-agent-memories]") ?? document; restoreSelectedMemoryItems(root); renderLocalDateTimes(root); } });
}
