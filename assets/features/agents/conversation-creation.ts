// Keep admission state outside forms that live SSE snapshots can replace.
const pending = new Map<string, { form: HTMLFormElement; started: boolean }>();
const buttons = new Map<HTMLButtonElement, { html: string; disabled: boolean; className: string }>();
let installed = false;

function creationForm(element: unknown): HTMLFormElement | null {
  const form = element instanceof Element ? element.closest<HTMLFormElement>("form") : null;
  if (!form || !/^\/agents\/[^/]+\/chat\/conversations$/.test(new URL(form.action, window.location.href).pathname)) return null;
  return form;
}

function key(form: HTMLFormElement): string {
  return new URL(form.action, window.location.href).pathname;
}

function refreshButtons() {
  document.querySelectorAll<HTMLFormElement>("form").forEach((candidate) => {
    const form = creationForm(candidate);
    if (!form || !pending.has(key(form))) return;
    form.querySelectorAll<HTMLButtonElement>('button[type="submit"], button:not([type])').forEach((button) => {
      if (!buttons.has(button)) buttons.set(button, { html: button.innerHTML, disabled: button.disabled, className: button.className });
      button.disabled = true;
      button.textContent = "Creating…";
      button.classList.remove("cursor-pointer");
      button.classList.add("cursor-wait", "opacity-50");
    });
  });
}

function reset(path?: string) {
  if (path) pending.delete(path);
  else pending.clear();
  buttons.forEach((original, button) => {
    const form = creationForm(button);
    if (form && pending.has(key(form))) return;
    button.innerHTML = original.html;
    button.disabled = original.disabled;
    button.className = original.className;
    buttons.delete(button);
  });
}

export function installConversationCreationGuard() {
  if (installed) return;
  installed = true;
  document.addEventListener("submit", (event) => {
    const form = creationForm(event.target);
    if (!form) return;
    if (pending.has(key(form))) {
      event.preventDefault();
      event.stopImmediatePropagation();
      return;
    }
    // Native validation runs before submit; retain it for manually dispatched events too.
    const submitter = event instanceof SubmitEvent ? event.submitter : null;
    const skipValidation = form.noValidate || (submitter instanceof HTMLButtonElement && submitter.formNoValidate);
    if (event.defaultPrevented || (!skipValidation && !form.checkValidity())) return;
    pending.set(key(form), { form, started: false });
    refreshButtons();
  }, true);
  document.addEventListener("htmx:beforeRequest", (event) => {
    const form = creationForm((event as CustomEvent<{ elt?: Element }>).detail?.elt);
    if (!form || event.defaultPrevented) return;
    const current = pending.get(key(form));
    if (current && (current.form !== form || current.started)) {
      event.preventDefault();
      return;
    }
    pending.set(key(form), { form, started: true });
    refreshButtons();
  });
  const finish = (event: Event) => {
    const form = creationForm((event as CustomEvent<{ elt?: Element }>).detail?.elt);
    if (form && pending.get(key(form))?.form === form) reset(key(form));
  };
  for (const name of ["htmx:afterRequest", "htmx:sendError", "htmx:timeout", "htmx:sendAbort", "htmx:validation:halted"]) document.addEventListener(name, finish);
  document.addEventListener("htmx:afterSwap", refreshButtons);
  document.addEventListener("htmx:sseMessage", refreshButtons);
  window.addEventListener("pageshow", () => reset());
}
