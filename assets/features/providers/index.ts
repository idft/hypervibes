import { csrfToken } from "../../core/csrf";
import { htmx } from "../../core/htmx";

export function initProviderAuthPrompts(root: ParentNode = document) {
  root.querySelectorAll<HTMLFormElement>('form[action$="/connect"]').forEach((form) => {
    if (form.dataset.providerAuthBound === "true") return;
    const prompts = Array.from(form.querySelectorAll<HTMLElement>("[data-provider-auth-prompt]"));
    if (prompts.length === 0) return;
    form.dataset.providerAuthBound = "true";
    const sync = () => {
      const values = new Map(prompts.map((prompt) => [prompt.dataset.providerAuthKey ?? "", prompt.querySelector<HTMLInputElement | HTMLSelectElement>("input, select")?.value ?? ""]));
      prompts.forEach((prompt) => {
        const key = prompt.dataset.providerAuthWhenKey;
        const expected = prompt.dataset.providerAuthWhenValue;
        const active = !key || expected === undefined || (prompt.dataset.providerAuthWhenOp === "neq" ? values.get(key) !== expected : values.get(key) === expected);
        prompt.classList.toggle("hidden", !active);
        prompt.querySelectorAll<HTMLInputElement | HTMLSelectElement>("input, select").forEach((control) => { control.disabled = !active; });
      });
    };
    form.addEventListener("input", sync); form.addEventListener("change", sync); sync();
  });
}

function initConnectModal(root: ParentNode) {
  const modal = root.querySelector<HTMLElement>("[data-provider-connect-modal]");
  const open = root.querySelector<HTMLButtonElement>("[data-provider-connect-open]");
  const selection = modal?.querySelector<HTMLElement>("[data-provider-connect-selection]");
  const step = modal?.querySelector<HTMLElement>("[data-provider-connect-step]");
  const loading = modal?.querySelector<HTMLElement>("[data-provider-connect-loading]");
  const form = modal?.querySelector<HTMLFormElement>("[data-provider-connect-form]");
  const provider = modal?.querySelector<HTMLSelectElement>("[data-provider-connect-provider]");
  const method = modal?.querySelector<HTMLSelectElement>("[data-provider-connect-method]");
  const submit = modal?.querySelector<HTMLButtonElement>("[data-provider-connect-submit]");
  const status = modal?.querySelector<HTMLElement>("[data-provider-connect-status]");
  if (!modal || !open || !selection || !step || !loading || !form || !provider || !method || !submit || !status || modal.dataset.bound === "true") return;
  modal.dataset.bound = "true";
  let pendingOAuthProvider: string | undefined;
  const cancelOAuth = (providerId: string) => void fetch(`/providers/${encodeURIComponent(providerId)}/connect/cancel`, { method: "POST", headers: { "X-CSRF-Token": csrfToken() ?? "" } });
  const sync = () => {
    const options = Array.from(method.options).filter((option) => option.dataset.providerId);
    options.forEach((option) => { option.hidden = option.dataset.providerId !== provider.value; });
    const available = options.filter((option) => option.dataset.providerId === provider.value && option.dataset.disabledReason === undefined);
    if (method.selectedOptions[0]?.hidden || method.selectedOptions[0]?.dataset.providerId !== provider.value || method.selectedOptions[0]?.dataset.disabledReason !== undefined) method.value = available.length === 1 ? available[0]?.value ?? "" : "";
    const selected = method.selectedOptions[0];
    const valid = Boolean(provider.value && method.value && selected?.dataset.disabledReason === undefined);
    submit.disabled = !valid; submit.classList.toggle("cursor-pointer", valid); submit.classList.toggle("cursor-not-allowed", !valid); submit.classList.toggle("opacity-50", !valid);
    method.closest("label")?.classList.toggle("hidden", available.length <= 1);
    status.textContent = selected?.dataset.disabledReason ?? (valid && selected?.dataset.providerAuthType === "api" ? "Use an API key from the provider, not a subscription sign-in." : "");
  };
  const reset = () => { selection.classList.remove("hidden"); step.classList.add("hidden"); loading.classList.add("hidden"); step.replaceChildren(); provider.value = ""; provider.disabled = false; method.value = ""; sync(); };
  const close = () => { const providerId = pendingOAuthProvider; pendingOAuthProvider = undefined; step.dispatchEvent(new Event("htmx:abort", { bubbles: true })); modal.classList.add("hidden"); modal.classList.remove("flex"); reset(); if (providerId) cancelOAuth(providerId); };
  const start = () => { selection.classList.add("hidden"); step.classList.add("hidden"); loading.classList.remove("hidden"); provider.disabled = true; };
  open.addEventListener("click", () => { modal.classList.remove("hidden"); modal.classList.add("flex"); provider.focus(); sync(); });
  provider.addEventListener("change", () => { selection.classList.remove("hidden"); step.classList.add("hidden"); step.replaceChildren(); sync(); }); method.addEventListener("change", sync);
  form.addEventListener("submit", (event) => {
    event.preventDefault(); if (submit.disabled || !provider.value || !method.value) return; start();
    const selected = method.selectedOptions[0];
    if (selected?.dataset.providerAuthType === "oauth" && selected.dataset.providerPromptCount === "0") { pendingOAuthProvider = provider.value; void htmx.ajax("post", `/providers/${encodeURIComponent(provider.value)}/connect?modal=true`, { target: step, swap: "innerHTML", values: { method: method.value } }); }
    else void htmx.ajax("get", `/providers/${encodeURIComponent(provider.value)}/connect?method=${encodeURIComponent(method.value)}&modal=true`, { target: step, swap: "innerHTML" });
  });
  modal.addEventListener("click", (event) => { const target = event.target as Element | null; if (target?.closest("[data-provider-connect-close]") || event.target === modal) close(); else if (target?.closest("[data-provider-connect-back]")) reset(); });
  document.addEventListener("keydown", (event) => { if (event.key === "Escape" && !modal.classList.contains("hidden")) close(); });
  document.addEventListener("htmx:afterSwap", (event) => { if ((event as CustomEvent<{ target: Element }>).detail.target !== step) return; loading.classList.add("hidden"); step.classList.remove("hidden"); const providerId = step.querySelector<HTMLElement>("[data-provider-oauth-pending]")?.dataset.providerId; if (providerId && modal.classList.contains("hidden")) cancelOAuth(providerId); else pendingOAuthProvider = providerId; });
  sync();
}

function initDisconnectModal(root: ParentNode) {
  const modal = root.querySelector<HTMLElement>("[data-provider-disconnect-modal]");
  const form = modal?.querySelector<HTMLFormElement>("[data-provider-disconnect-form]");
  const name = modal?.querySelector<HTMLElement>("[data-provider-disconnect-name]");
  if (!modal || !form || !name || modal.dataset.bound === "true") return;
  modal.dataset.bound = "true";
  const close = () => { modal.classList.add("hidden"); modal.classList.remove("flex"); };
  document.addEventListener("click", (event) => { const target = event.target as Element | null; const trigger = target?.closest<HTMLElement>("[data-provider-disconnect-trigger]"); if (trigger) { form.action = trigger.dataset.providerDisconnectAction ?? ""; name.textContent = trigger.dataset.providerDisconnectName ?? "this provider"; modal.classList.remove("hidden"); modal.classList.add("flex"); } else if (target?.closest("[data-provider-disconnect-close]") || event.target === modal) close(); });
  document.addEventListener("keydown", (event) => { if (event.key === "Escape" && !modal.classList.contains("hidden")) close(); });
}

export function initProviders(root: ParentNode = document) {
  initProviderAuthPrompts(root); initConnectModal(root); initDisconnectModal(root);
}
