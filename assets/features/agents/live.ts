import { animateNumberRolls, tickRunningDurations } from "../../shared/presentation";
import { installConversationCreationGuard } from "./conversation-creation";
import { installChatPermissions } from "./chat-permissions";

const SUB_AGENTS_REFRESH_KEY = "agent-sub-agents-refresh-required";

const BOTTOM_PIN_THRESHOLD_PX = 40;
const pinnedToBottom = new WeakSet<HTMLElement>();

function nearTranscriptBottom(scroll: HTMLElement) {
  return scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < BOTTOM_PIN_THRESHOLD_PX;
}

function scrollTranscriptIfPinned(scroll: HTMLElement) {
  if (!pinnedToBottom.has(scroll)) return;
  scroll.scrollTop = scroll.scrollHeight;
}

function scrollRunTranscriptToBottom() {
  document.querySelectorAll<HTMLElement>("[data-run-transcript-scroll], [data-conversation-transcript-scroll]").forEach(scrollTranscriptIfPinned);
}

function pinTranscriptToBottom(scroll: HTMLElement) {
  pinnedToBottom.add(scroll);
  scroll.scrollTop = scroll.scrollHeight;
}

export function initRunTranscripts(root: ParentNode = document) {
  if (typeof ResizeObserver === "undefined") return;
  root.querySelectorAll<HTMLElement>("[data-run-transcript-scroll], [data-conversation-transcript-scroll]").forEach((scroll) => {
    const transcript = scroll.querySelector<HTMLElement>('[sse-swap="run-transcript"], [sse-swap="conversation-transcript"]');
    if (!transcript || scroll.dataset.bound === "true") return;
    scroll.dataset.bound = "true";
    scroll.addEventListener("scroll", () => {
      if (nearTranscriptBottom(scroll)) pinnedToBottom.add(scroll);
      else pinnedToBottom.delete(scroll);
    });
    const observer = new ResizeObserver(() => scrollTranscriptIfPinned(scroll));
    observer.observe(scroll); observer.observe(transcript);
    pinTranscriptToBottom(scroll);
  });
  scrollRunTranscriptToBottom();
}

function updateModelDependentButtons(form: HTMLFormElement) {
  const input = form.querySelector<HTMLInputElement>('input[name="model_selection"]');
  if (!input || !/\/agents\/[^/]+\/sub-agents\/\d+\/model$/.test(new URL(form.action, window.location.href).pathname)) return;
  const hasModel = Boolean(input.value.trim());
  document.querySelectorAll<HTMLButtonElement>("[data-model-dependent-run-now]").forEach((button) => { button.disabled = !hasModel; button.classList.toggle("cursor-pointer", hasModel); button.classList.toggle("cursor-not-allowed", !hasModel); button.classList.toggle("opacity-50", !hasModel); button.title = hasModel ? "" : "No model set"; });
  document.querySelectorAll<HTMLButtonElement>("[data-model-dependent-enable]").forEach((button) => {
    const disabled = button.dataset.enabled !== "true" && !hasModel;
    button.disabled = disabled;
    button.classList.toggle("cursor-pointer", !disabled);
    button.classList.toggle("cursor-not-allowed", disabled);
    button.classList.toggle("opacity-50", disabled);
    if (button.dataset.enabled !== "true") {
      button.classList.toggle("border-zinc-800", disabled);
      button.classList.toggle("text-zinc-500", disabled);
      button.classList.toggle("border-emerald-900/60", !disabled);
      button.classList.toggle("bg-emerald-950/20", !disabled);
      button.classList.toggle("text-emerald-300", !disabled);
      button.classList.toggle("hover:border-emerald-800", !disabled);
      button.classList.toggle("hover:bg-emerald-950/40", !disabled);
      button.classList.toggle("hover:text-emerald-200", !disabled);
    }
    button.title = disabled ? "No model set" : "";
  });
}

function installDetailDeleteModal() {
  const close = (modal: HTMLElement) => { modal.classList.add("hidden"); modal.classList.remove("flex"); };
  document.addEventListener("click", (event) => {
    const target = event.target as Element | null;
    const trigger = target?.closest<HTMLElement>("[data-detail-delete-trigger]");
    const modal = document.getElementById("detail-delete-modal");
    if (trigger && modal) {
      const form = document.getElementById("detail-delete-form") as HTMLFormElement | null;
      const kind = trigger.dataset.deleteKind ?? "sub-agent";
      if (form) form.action = trigger.dataset.deleteAction ?? "";
      const title = document.getElementById("detail-delete-modal-title"); const body = document.getElementById("detail-delete-modal-body"); const confirm = document.getElementById("confirm-detail-delete-btn");
      if (title) title.textContent = `Delete ${kind}`;
      if (body) body.textContent = kind === "sub-agent"
        ? `Delete ${trigger.dataset.deleteLabel ?? kind}? This permanently removes the sub-agent, its run history, and transcript sessions.`
        : `Are you sure you want to delete ${trigger.dataset.deleteLabel ?? kind}? This action cannot be undone.`;
      if (confirm) confirm.textContent = `Delete ${kind}`;
      modal.classList.remove("hidden"); modal.classList.add("flex");
    } else if (modal && (target?.closest("#cancel-detail-delete-btn") || target === modal)) close(modal);
  });
  document.addEventListener("keydown", (event) => { if (event.key === "Escape") { const modal = document.getElementById("detail-delete-modal"); if (modal?.classList.contains("flex")) close(modal); } });
}

function installPositionCloseModal() {
  const close = (modal: HTMLElement) => { modal.classList.add("hidden"); modal.classList.remove("flex"); };
  document.addEventListener("click", (event) => {
    const target = event.target as Element | null;
    const modal = document.getElementById("position-close-modal");
    const trigger = target?.closest<HTMLElement>("[data-position-close-trigger]");
    if (trigger && modal) {
      const form = document.getElementById("position-close-form") as HTMLFormElement | null;
      const body = document.getElementById("position-close-modal-body");
      const confirm = document.getElementById("position-close-confirm");
      if (form) form.action = trigger.dataset.positionCloseAction ?? "";
      if (body) body.textContent = trigger.dataset.positionCloseMessage ?? "Close this position at market?";
      if (confirm) confirm.textContent = trigger.dataset.positionCloseConfirm ?? "Close";
      modal.classList.remove("hidden"); modal.classList.add("flex");
    } else if (modal && (target?.closest("[data-position-close-cancel]") || target === modal)) close(modal);
  });
  document.addEventListener("keydown", (event) => { if (event.key === "Escape") { const modal = document.getElementById("position-close-modal"); if (modal?.classList.contains("flex")) close(modal); } });
}

function installRunCancelModal() {
  const close = (modal: HTMLElement) => { modal.classList.add("hidden"); modal.classList.remove("flex"); };
  document.addEventListener("click", (event) => {
    const target = event.target as Element | null;
    const modal = document.getElementById("run-cancel-modal");
    if (target?.closest("[data-run-cancel-trigger]") && modal) {
      modal.classList.remove("hidden"); modal.classList.add("flex");
    } else if (modal && (target?.closest("[data-run-cancel-close]") || target === modal)) {
      close(modal);
    }
  });
  document.addEventListener("keydown", (event) => { if (event.key === "Escape") { const modal = document.getElementById("run-cancel-modal"); if (modal?.classList.contains("flex")) close(modal); } });
}

function installConversationComposerShortcut() {
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.shiftKey || event.isComposing) return;
    const textarea = event.target instanceof HTMLTextAreaElement ? event.target : null;
    const form = textarea?.closest<HTMLFormElement>('form[action*="/chat/"][action$="/messages"]');
    if (!textarea || !form || textarea.disabled) return;
    event.preventDefault();
    if (!textarea.value.trim()) return;
    form.requestSubmit();
  });
}

function followConversationTranscriptOnSend(event: Event) {
  const form = event.target;
  if (!(form instanceof HTMLFormElement) || !form.matches('form[action*="/chat/"][action$="/messages"]')) return;
  const textarea = form.querySelector<HTMLTextAreaElement>('textarea[name="message"]');
  if (textarea && !textarea.value.trim()) return;
  document.querySelectorAll<HTMLElement>("[data-conversation-transcript-scroll]").forEach(pinTranscriptToBottom);
}

function preserveConversationDraftOnSse(event: Event) {
  const composer = event.target;
  if (!(composer instanceof HTMLElement) || composer.id !== "conversation-composer") return;
  const textarea = composer.querySelector<HTMLTextAreaElement>('textarea[name="message"]');
  if (!textarea || (!textarea.value && document.activeElement !== textarea)) return;

  // SSE snapshots contain an empty composer. Keep the live textarea (and its
  // focus/caret) while still applying the server's control state (busy Send/
  // Queue labeling or initializing-disabled controls) to the rest of the form.
  const data = (event as CustomEvent<{ data?: string }>).detail?.data;
  if (typeof data !== "string") return;
  const fragment = document.createElement("template");
  fragment.innerHTML = data;
  const nextTextarea = fragment.content.querySelector<HTMLTextAreaElement>('textarea[name="message"]');
  const button = composer.querySelector<HTMLButtonElement>('button[type="submit"]');
  const nextButton = fragment.content.querySelector<HTMLButtonElement>('button[type="submit"]');
  if (!nextTextarea || !button || !nextButton) return;

  event.preventDefault();
  textarea.disabled = nextTextarea.disabled;
  button.disabled = nextButton.disabled;
  button.className = nextButton.className;
  button.innerHTML = nextButton.innerHTML;
  const hint = composer.querySelector<HTMLElement>("[data-conversation-composer-hint]");
  const nextHint = fragment.content.querySelector<HTMLElement>("[data-conversation-composer-hint]");
  if (hint && nextHint) hint.textContent = nextHint.textContent;
}

export function installAgentLiveLifecycle() {
  installChatPermissions();
  installConversationCreationGuard();
  installDetailDeleteModal();
  installPositionCloseModal();
  installRunCancelModal();
  installConversationComposerShortcut();
  document.addEventListener("submit", followConversationTranscriptOnSend);
  document.addEventListener("htmx:sseBeforeMessage", preserveConversationDraftOnSse);
  document.addEventListener("change", (event) => {
    const input = event.target;
    if (!(input instanceof HTMLInputElement) || input.name !== "model_selection") return;
    const form = input.closest<HTMLFormElement>("form");
    if (form) updateModelDependentButtons(form);
  });
  document.addEventListener("htmx:sseMessage", (event) => {
    const detail = (event as CustomEvent<{ type?: string; event?: Event }>).detail;
    if (detail.type === "balance" || detail.type === "positions") window.setTimeout(animateNumberRolls, 50);
  });
  document.addEventListener("htmx:afterSwap", (event) => { const target = (event as CustomEvent<{ target?: unknown }>).detail.target; if (!(target instanceof Element)) return; initRunTranscripts(target); if (target.matches('[sse-swap="run-summary"]')) tickRunningDurations(); if (target.matches('[sse-swap="run-transcript"], [sse-swap="conversation-transcript"]') || target.querySelector('[sse-swap="run-transcript"], [sse-swap="conversation-transcript"]')) scrollRunTranscriptToBottom(); });
  document.addEventListener("htmx:afterSettle", (event) => { const target = (event as CustomEvent<{ target?: unknown }>).detail.target; if (!(target instanceof Element)) return; if (target.matches('[sse-swap="run-transcript"], [sse-swap="conversation-transcript"]') || target.querySelector('[sse-swap="run-transcript"], [sse-swap="conversation-transcript"]')) scrollRunTranscriptToBottom(); });
  document.addEventListener("htmx:afterRequest", (event) => {
    const detail = (event as CustomEvent<{ successful?: boolean; elt?: Element }>).detail;
    if (!detail.successful || !(detail.elt instanceof HTMLFormElement)) return;
    if (/^\/agents\/[^/]+\/sub-agents\/\d+\/model$/.test(new URL(detail.elt.action, window.location.href).pathname)) window.sessionStorage.setItem(SUB_AGENTS_REFRESH_KEY, "true");
    updateModelDependentButtons(detail.elt);
  });
  window.addEventListener("pageshow", (event) => { const rail = document.querySelector(".agent-rail"); rail?.classList.remove("agent-rail-exit"); if (event.persisted && document.querySelector("[data-agent-sub-agents]") && window.sessionStorage.getItem(SUB_AGENTS_REFRESH_KEY) === "true") window.location.reload(); });
}
