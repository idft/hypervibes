import { animateNumberRolls, tickRunningDurations } from "../../shared/presentation";

const JOBS_REFRESH_KEY = "agent-jobs-refresh-required";

export function scrollRunTranscriptToBottom() {
  document.querySelectorAll<HTMLElement>("[data-run-transcript-scroll]").forEach((scroll) => { scroll.scrollTop = scroll.scrollHeight; });
}

export function initRunTranscripts(root: ParentNode = document) {
  if (typeof ResizeObserver === "undefined") return;
  root.querySelectorAll<HTMLElement>("[data-run-transcript-scroll]").forEach((scroll) => {
    const transcript = scroll.querySelector<HTMLElement>('[sse-swap="run-transcript"]');
    if (!transcript || scroll.dataset.bound === "true") return;
    scroll.dataset.bound = "true";
    const observer = new ResizeObserver(scrollRunTranscriptToBottom);
    observer.observe(scroll); observer.observe(transcript);
  });
  scrollRunTranscriptToBottom();
}

function updateModelDependentButtons(form: HTMLFormElement) {
  const input = form.querySelector<HTMLInputElement>('input[name="model_selection"]');
  if (!input || !/\/agents\/[^/]+\/(?:jobs|hooks)\/\d+\/model$/.test(new URL(form.action, window.location.href).pathname)) return;
  const hasModel = Boolean(input.value.trim());
  document.querySelectorAll<HTMLButtonElement>("[data-model-dependent-run-now]").forEach((button) => { button.disabled = !hasModel; button.classList.toggle("cursor-pointer", hasModel); button.classList.toggle("cursor-not-allowed", !hasModel); button.classList.toggle("opacity-50", !hasModel); button.title = hasModel ? "" : "No model set"; });
  document.querySelectorAll<HTMLButtonElement>("[data-model-dependent-enable]").forEach((button) => { const disabled = button.dataset.enabled !== "true" && !hasModel; button.disabled = disabled; button.classList.toggle("cursor-pointer", !disabled); button.classList.toggle("cursor-not-allowed", disabled); button.classList.toggle("opacity-50", disabled); button.title = disabled ? "No model set" : ""; });
}

function installDetailDeleteModal() {
  const close = (modal: HTMLElement) => { modal.classList.add("hidden"); modal.classList.remove("flex"); };
  document.addEventListener("click", (event) => {
    const target = event.target as Element | null;
    const trigger = target?.closest<HTMLElement>("[data-detail-delete-trigger]");
    const modal = document.getElementById("detail-delete-modal");
    if (trigger && modal) {
      const form = document.getElementById("detail-delete-form") as HTMLFormElement | null;
      const kind = trigger.dataset.deleteKind ?? "job";
      if (form) form.action = trigger.dataset.deleteAction ?? "";
      const title = document.getElementById("detail-delete-modal-title"); const body = document.getElementById("detail-delete-modal-body"); const confirm = document.getElementById("confirm-detail-delete-btn");
      if (title) title.textContent = `Delete ${kind}`; if (body) body.textContent = `Are you sure you want to delete ${trigger.dataset.deleteLabel ?? kind}? This action cannot be undone.`; if (confirm) confirm.textContent = `Delete ${kind}`;
      modal.classList.remove("hidden"); modal.classList.add("flex");
    } else if (modal && (target?.closest("#cancel-detail-delete-btn") || target === modal)) close(modal);
  });
  document.addEventListener("keydown", (event) => { if (event.key === "Escape") { const modal = document.getElementById("detail-delete-modal"); if (modal?.classList.contains("flex")) close(modal); } });
}

export function installAgentLiveLifecycle() {
  installDetailDeleteModal();
  document.addEventListener("htmx:sseMessage", (event) => {
    const detail = (event as CustomEvent<{ type?: string; event?: Event }>).detail;
    if (detail.type === "balance" || detail.type === "positions") window.setTimeout(animateNumberRolls, 50);
  });
  document.addEventListener("htmx:afterSwap", (event) => { const target = (event as CustomEvent<{ target?: unknown }>).detail.target; if (!(target instanceof Element)) return; initRunTranscripts(target); if (target.matches('[sse-swap="run-summary"]')) tickRunningDurations(); if (target.matches('[sse-swap="run-transcript"]') || target.querySelector('[sse-swap="run-transcript"]')) scrollRunTranscriptToBottom(); });
  document.addEventListener("htmx:afterSettle", (event) => { const target = (event as CustomEvent<{ target?: unknown }>).detail.target; if (!(target instanceof Element)) return; if (target.matches('[sse-swap="run-transcript"]') || target.querySelector('[sse-swap="run-transcript"]')) scrollRunTranscriptToBottom(); });
  document.addEventListener("htmx:afterRequest", (event) => {
    const detail = (event as CustomEvent<{ successful?: boolean; elt?: Element }>).detail;
    if (!detail.successful || !(detail.elt instanceof HTMLFormElement)) return;
    if (/^\/agents\/[^/]+\/(?:jobs|hooks)\/\d+\/model$/.test(new URL(detail.elt.action, window.location.href).pathname)) window.sessionStorage.setItem(JOBS_REFRESH_KEY, "true");
    updateModelDependentButtons(detail.elt);
  });
  window.addEventListener("pageshow", (event) => { const rail = document.querySelector(".agent-rail"); rail?.classList.remove("agent-rail-exit"); if (event.persisted && document.querySelector("[data-agent-jobs]") && window.sessionStorage.getItem(JOBS_REFRESH_KEY) === "true") window.location.reload(); });
}
