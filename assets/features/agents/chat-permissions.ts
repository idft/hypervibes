import htmx from "htmx.org";

let installed = false;
const pendingSummaries = new WeakMap<HTMLDialogElement, string>();

export function installChatPermissions() {
  if (installed) return;
  installed = true;

  document.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const dialog = document.getElementById("chat-permissions-modal");
    if (!(dialog instanceof HTMLDialogElement)) return;
    const form = dialog.querySelector<HTMLFormElement>("[data-chat-permissions-form]");
    if (target.closest("[data-chat-permissions-open]")) {
      if (dialog.open) return;
      form?.reset();
      dialog.addEventListener("close", () => {
        form?.reset();
        const pending = pendingSummaries.get(dialog);
        pendingSummaries.delete(dialog);
        const summary = dialog.closest<HTMLElement>("#conversation-summary");
        if (pending && summary?.isConnected) {
          htmx.swap(summary, pending, { swapStyle: "innerHTML", swapDelay: 0, settleDelay: 0 });
          summary.querySelector<HTMLButtonElement>("[data-chat-permissions-open]")?.focus();
        }
      }, { once: true });
      dialog.showModal();
    } else if (target.closest("[data-chat-permissions-cancel]") || target === dialog) {
      dialog.close();
    }
  });

  document.addEventListener("htmx:sseBeforeMessage", (event) => {
    const summary = event.target;
    if (!(summary instanceof HTMLElement) || summary.id !== "conversation-summary") return;
    const dialog = summary.querySelector<HTMLDialogElement>("#chat-permissions-modal");
    const data = (event as CustomEvent<{ data?: string }>).detail?.data;
    if (!dialog?.open || typeof data !== "string") return;

    // Keep the dialog and its unsaved edits intact during live updates. Apply
    // the newest snapshot after dismissal, but honor the busy state immediately.
    event.preventDefault();
    pendingSummaries.set(dialog, data);
    const fragment = document.createElement("template");
    fragment.innerHTML = data;
    const disabled = fragment.content.querySelector<HTMLButtonElement>("[data-chat-permissions-open]")?.disabled ?? true;
    const fieldset = dialog.querySelector("fieldset");
    const save = dialog.querySelector<HTMLButtonElement>('button[type="submit"]');
    if (fieldset) fieldset.disabled = disabled;
    if (save) save.disabled = disabled;
  });
}
