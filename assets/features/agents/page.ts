import { csrfToken } from "../../core/csrf";
import { initClickableRows } from "../../shared/clickable-rows";

function initAgentCreation(root: ParentNode) {
  const page = root.querySelector<HTMLElement>("[data-agent-creation]");
  const button = page?.querySelector<HTMLButtonElement>("[data-create-new-agent-subaccount]");
  const status = page?.querySelector<HTMLElement>("[data-new-agent-subaccount-status]");
  if (!page || !button || !status || page.dataset.creationBound === "true") return;
  page.dataset.creationBound = "true";
  button.addEventListener("click", () => {
    const name = page.querySelector<HTMLInputElement>("#display_name");
    const displayName = name?.value.trim() ?? "";
    if (!displayName) { name?.focus(); status.textContent = "Enter an agent name first."; return; }
    button.disabled = true; status.textContent = "Creating Sub-Account with the server trading signer...";
    void fetch("/account/subaccounts", { method: "POST", headers: { "content-type": "application/json", "X-CSRF-Token": csrfToken() ?? "" }, body: JSON.stringify({ displayName }) })
      .then(async (response) => { const result = (await response.json().catch(() => null)) as { name?: string; created?: boolean; error?: string } | null; if (!response.ok || !result?.name) throw new Error(result?.error ?? "Sub-Account creation failed."); status.textContent = result.created ? "Sub-Account created and selected below." : "Existing Sub-Account selected below."; page.querySelector<HTMLElement>("[data-new-agent-subaccount-refresh]")?.dispatchEvent(new CustomEvent("newAgentSubaccountCreated", { detail: { name: result.name } })); })
      .catch((reason: unknown) => { status.textContent = reason instanceof Error ? reason.message : "Sub-Account creation failed."; })
      .finally(() => { button.disabled = false; });
  });
}

function initPromptEditor(root: ParentNode) {
  root.querySelectorAll<HTMLElement>("[data-agent-prompts]").forEach((container) => {
    if (container.dataset.bound === "true") return;
    const forms = Array.from(container.querySelectorAll<HTMLFormElement>("[data-agent-prompt-form]"));
    if (forms.length === 0) return;
    container.dataset.bound = "true";
    container.querySelectorAll<HTMLElement>("[data-agent-prompt-nav-item]").forEach((item) => item.addEventListener("click", () => selectPrompt(container, item.dataset.agentPromptNavItem ?? "")));
    forms.forEach((form) => {
      const key = form.dataset.agentPromptForm ?? "";
      const textarea = form.querySelector<HTMLTextAreaElement>('textarea[name="prompt"]');
      const save = container.querySelector<HTMLButtonElement>(`[data-agent-prompt-save="${key}"]`);
      const importButton = container.querySelector<HTMLButtonElement>(`[data-agent-prompt-import="${key}"]`);
      const file = container.querySelector<HTMLInputElement>(`[data-agent-prompt-file-input="${key}"]`);
      const reset = container.querySelector<HTMLButtonElement>(`[data-agent-prompt-reset="${key}"]`);
      const fallback = container.querySelector<HTMLTextAreaElement>(`#default-${key}-strategy-prompt-value`);
      if (!textarea || !save || !importButton || !file || !reset || !fallback) return;
      const original = textarea.value;
      const update = () => { const dirty = textarea.value !== original; save.disabled = !dirty; save.classList.toggle("hidden", !dirty); };
      textarea.addEventListener("input", update); reset.addEventListener("click", () => { textarea.value = fallback.value; update(); }); importButton.addEventListener("click", () => file.click());
      file.addEventListener("change", () => { const selected = file.files?.[0]; file.value = ""; if (selected) void selected.text().then((contents) => { textarea.value = contents; update(); }); }); update();
    });
    selectPrompt(container, container.dataset.activePrompt ?? forms[0].dataset.agentPromptForm ?? "");
  });
}

function selectPrompt(container: HTMLElement, key: string) {
  container.dataset.activePrompt = key;
  container.querySelectorAll<HTMLElement>("[data-agent-prompt-nav-item]").forEach((item) => {
    const active = item.dataset.agentPromptNavItem === key;
    item.classList.toggle("border-zinc-700", active);
    item.classList.toggle("bg-zinc-900", active);
    item.classList.toggle("text-white", active);
  });
  container.querySelectorAll<HTMLElement>("[data-agent-prompt-form]").forEach((form) => {
    form.classList.toggle("hidden", form.dataset.agentPromptForm !== key);
  });
}

function initInstrumentSelectors(root: ParentNode) {
  root.querySelectorAll<HTMLElement>("[data-agent-instrument-selector]").forEach((container) => {
    if (container.dataset.bound === "true") return;
    const search = container.querySelector<HTMLInputElement>("[data-instrument-search]");
    const list = container.querySelector<HTMLElement>("[data-selected-instrument-list]");
    const empty = container.querySelector<HTMLElement>("[data-selected-instrument-empty]");
    const warning = container.querySelector<HTMLElement>("[data-no-currencies-warning]");
    const searchEmpty = container.querySelector<HTMLElement>("[data-instrument-search-empty]");
    const scrollContainer = container.querySelector<HTMLElement>("[data-instrument-scroll-container]");
    const selectButton = container.querySelector<HTMLButtonElement>("[data-select-currencies]");
    const modal = container.querySelector<HTMLElement>("[data-currency-modal]");
    const apply = container.querySelector<HTMLButtonElement>("[data-apply-currencies]");
    const selectAll = container.querySelector<HTMLButtonElement>("[data-select-all-currencies]");
    const selectNone = container.querySelector<HTMLButtonElement>("[data-select-no-currencies]");
    const closeButtons = container.querySelectorAll<HTMLButtonElement>("[data-currency-modal-close]");
    const rows = Array.from(container.querySelectorAll<HTMLElement>("[data-instrument-row]"));
    if (!list || !empty || !warning || !selectButton || !modal || !apply || !selectAll || !selectNone || rows.length === 0) return;
    container.dataset.bound = "true";
    const inputFor = (row: HTMLElement) => row.querySelector<HTMLInputElement>('input[name="instrument_id"]');
    const labelFor = (row: HTMLElement) => inputFor(row)?.value ?? "";
    const loadLogo = (row: HTMLElement) => {
      const image = row.querySelector<HTMLImageElement>("[data-instrument-logo]");
      const url = row.dataset.instrumentLogoUrl;
      if (!image || !url || image.hasAttribute("src") || row.dataset.logoLoaded === "true") return;
      row.dataset.logoLoaded = "true";
      image.src = url;
    };
    const sync = () => {
      const selected = rows.filter((row) => inputFor(row)?.checked);
      list.replaceChildren();
      selected.forEach((row) => {
        const label = labelFor(row);
        const item = document.createElement("div");
        const identity = document.createElement("div");
        const logo = document.createElement("img");
        const name = document.createElement("span");
        item.className = "flex shrink-0 items-center gap-2 rounded-xl border border-zinc-800 bg-zinc-950/70 px-3 py-2";
        identity.className = "flex items-center gap-2";
        logo.alt = "";
        logo.setAttribute("aria-hidden", "true");
        logo.className = "h-5 w-5 rounded-sm bg-zinc-800";
        if (row.dataset.instrumentLogoUrl) logo.src = row.dataset.instrumentLogoUrl;
        name.className = "font-medium text-zinc-200";
        name.textContent = label;
        identity.append(logo, name);
        item.append(identity);
        list.append(item);
      });
      empty.classList.toggle("hidden", selected.length > 0); warning.classList.toggle("hidden", selected.length > 0);
    };
    const filter = () => { const query = search?.value.trim().toLowerCase() ?? ""; let count = 0; rows.forEach((row) => { const visible = !query || (row.dataset.instrumentLabel ?? "").toLowerCase().includes(query); row.classList.toggle("hidden", !visible); if (visible) count += 1; }); searchEmpty?.classList.toggle("hidden", count > 0); };
    let selectionBeforeModal: boolean[] = [];
    const closeModal = (restoreSelection: boolean) => {
      if (restoreSelection) rows.forEach((row, index) => { const input = inputFor(row); if (input) input.checked = selectionBeforeModal[index] ?? false; });
      modal.classList.add("hidden");
      modal.classList.remove("flex");
      document.body.classList.remove("overflow-hidden");
    };
    const openModal = () => {
      selectionBeforeModal = rows.map((row) => inputFor(row)?.checked ?? false);
      if (search) search.value = "";
      filter();
      modal.classList.remove("hidden");
      modal.classList.add("flex");
      document.body.classList.add("overflow-hidden");
      search?.focus();
    };
    search?.addEventListener("input", filter);
    selectButton.addEventListener("click", openModal);
    closeButtons.forEach((button) => button.addEventListener("click", () => closeModal(true)));
    apply.addEventListener("click", () => { sync(); closeModal(false); });
    selectAll.addEventListener("click", () => rows.forEach((row) => { const input = inputFor(row); if (input) input.checked = true; }));
    selectNone.addEventListener("click", () => rows.forEach((row) => { const input = inputFor(row); if (input) input.checked = false; }));
    document.addEventListener("keydown", (event) => { if (event.key === "Escape" && !modal.classList.contains("hidden")) closeModal(true); });
    if (scrollContainer && "IntersectionObserver" in window) {
      const observer = new IntersectionObserver((entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting && entry.target instanceof HTMLElement) {
            loadLogo(entry.target);
            observer.unobserve(entry.target);
          }
        });
      }, { root: scrollContainer, rootMargin: "80px 0px", threshold: 0 });
      rows.forEach((row) => observer.observe(row));
    } else {
      rows.forEach(loadLogo);
    }
    sync(); filter();
  });
}

function installModal(trigger: string, modalId: string, cancelId: string, configure?: (button: HTMLElement) => void) {
  document.addEventListener("click", (event) => {
    const target = event.target as Element | null;
    const button = target?.closest<HTMLElement>(trigger);
    const modal = document.getElementById(modalId);
    if (!modal) return;
    if (button) { configure?.(button); modal.classList.remove("hidden"); modal.classList.add("flex"); }
    if (target?.closest(`#${cancelId}`) || target === modal) { modal.classList.add("hidden"); modal.classList.remove("flex"); }
  });
  document.addEventListener("keydown", (event) => { if (event.key === "Escape") document.getElementById(modalId)?.classList.add("hidden"); });
}

function installAgentModals() {
  installModal("[data-delete-agent-trigger]", "delete-modal", "cancel-delete-btn");
  installModal("[data-agent-job-delete-trigger]", "agent-job-delete-modal", "cancel-agent-job-delete-btn", (button) => { const form = document.getElementById("agent-job-delete-form") as HTMLFormElement | null; const kind = button.dataset.deleteKind ?? "job"; if (form) form.action = button.dataset.deleteAction ?? ""; const title = document.getElementById("agent-job-delete-modal-title"); const confirm = document.getElementById("confirm-agent-job-delete-btn"); const body = document.getElementById("agent-job-delete-modal-body"); if (title) title.textContent = `Delete ${kind}`; if (confirm) confirm.textContent = `Delete ${kind}`; if (body) body.textContent = `Are you sure you want to delete ${button.dataset.deleteLabel ?? kind}? This action cannot be undone.`; });
  installModal("[data-regenerate-workspace-trigger]", "regenerate-workspace-modal", "cancel-regenerate-workspace-btn");
}

function initInlineEditors(root: ParentNode) {
  root.querySelectorAll<HTMLElement>("[data-inline-editor], [data-timeout-editor]").forEach((editor) => {
    if (editor.dataset.bound === "true") return; editor.dataset.bound = "true";
    const display = editor.querySelector<HTMLElement>("[data-inline-editor-display-row], [data-timeout-display-row]");
    const form = editor.querySelector<HTMLElement>("[data-inline-editor-form], [data-timeout-edit-form]");
    const trigger = editor.querySelector<HTMLElement>("[data-inline-editor-trigger], [data-timeout-edit-trigger]");
    const cancel = editor.querySelector<HTMLElement>("[data-inline-editor-cancel], [data-timeout-cancel]");
    const input = editor.querySelector<HTMLInputElement>("[data-inline-editor-input], [data-timeout-input]");
    if (!display || !form || !trigger || !cancel || !input) return;
    const close = () => { display.classList.remove("hidden"); form.classList.add("hidden"); };
    const open = () => { document.querySelectorAll<HTMLElement>("[data-inline-editor], [data-timeout-editor]").forEach((other) => { if (other !== editor) { other.querySelector<HTMLElement>("[data-inline-editor-display-row], [data-timeout-display-row]")?.classList.remove("hidden"); other.querySelector<HTMLElement>("[data-inline-editor-form], [data-timeout-edit-form]")?.classList.add("hidden"); } }); display.classList.add("hidden"); form.classList.remove("hidden"); input.focus(); input.select(); };
    trigger.addEventListener("click", (event) => { event.preventDefault(); open(); }); cancel.addEventListener("click", (event) => { event.preventDefault(); close(); }); input.addEventListener("keydown", (event) => { if (event.key === "Escape") { event.preventDefault(); close(); } }); if (editor.hasAttribute("data-inline-editor-open") || editor.hasAttribute("data-timeout-editor-open")) open();
  });
}

function initJobDetailModals(root: ParentNode) {
  root.querySelectorAll<HTMLElement>("[data-job-detail-modal]").forEach((modal) => {
    if (modal.dataset.bound === "true") return;
    const trigger = root.querySelector<HTMLElement>(`[data-job-detail-modal-trigger="${modal.id}"]`);
    const initialFocus = modal.querySelector<HTMLElement>("[data-job-detail-modal-initial-focus]");
    if (!trigger || !initialFocus) return;
    modal.dataset.bound = "true";
    const close = () => {
      modal.classList.add("hidden");
      modal.classList.remove("flex");
      modal.setAttribute("aria-hidden", "true");
      document.body.classList.remove("overflow-hidden");
      trigger.focus();
    };
    const open = () => {
      modal.classList.remove("hidden");
      modal.classList.add("flex");
      modal.setAttribute("aria-hidden", "false");
      document.body.classList.add("overflow-hidden");
      initialFocus.focus();
    };
    trigger.addEventListener("click", open);
    modal.querySelectorAll<HTMLElement>("[data-job-detail-modal-close]").forEach((button) => button.addEventListener("click", close));
    modal.addEventListener("keydown", (event) => { if (event.key === "Escape") { event.preventDefault(); close(); } });
  });
}

function initWorkspaceModal(root: ParentNode) {
  const modal = root.querySelector<HTMLElement>("#regenerate-workspace-modal");
  const hardReset = modal?.querySelector<HTMLInputElement>("#hard-reset-workspace");
  const resetMemories = modal?.querySelector<HTMLInputElement>("#reset-memories");
  const option = modal?.querySelector<HTMLElement>("#reset-memories-option");
  const title = modal?.querySelector<HTMLElement>("#reset-memories-title");
  const description = modal?.querySelector<HTMLElement>("#reset-memories-description");
  if (!modal || !hardReset || !resetMemories || !option || !title || !description || modal.dataset.workspaceBound === "true") return;
  modal.dataset.workspaceBound = "true";
  const sync = () => {
    const enabled = hardReset.checked;
    resetMemories.disabled = !enabled;
    if (!enabled) resetMemories.checked = false;
    option.classList.toggle("text-zinc-500", !enabled); option.classList.toggle("text-zinc-200", enabled);
    title.classList.toggle("text-zinc-400", !enabled); title.classList.toggle("text-white", enabled);
    description.classList.toggle("text-zinc-600", !enabled); description.classList.toggle("text-zinc-400", enabled);
  };
  hardReset.addEventListener("change", sync);
  sync();
}

export function initAgentPage(root: ParentNode = document) { initAgentCreation(root); initPromptEditor(root); initInstrumentSelectors(root); initClickableRows(root); initInlineEditors(root); initJobDetailModals(root); initWorkspaceModal(root); }
export function installAgentPageLifecycle() {
  installAgentModals();
  document.addEventListener("click", (event) => {
    const item = (event.target as Element | null)?.closest<HTMLElement>("[data-agent-prompt-nav-item]");
    const container = item?.closest<HTMLElement>("[data-agent-prompts]");
    if (item && container) selectPrompt(container, item.dataset.agentPromptNavItem ?? "");
  });
  document.addEventListener("htmx:load", (event) => {
    const root = (event as CustomEvent<{ elt?: unknown }>).detail.elt;
    if (root instanceof Element) initAgentPage(root);
  });
  document.addEventListener("htmx:afterSwap", (event) => {
    const target = (event as CustomEvent<{ target?: unknown }>).detail.target;
    if (target instanceof Element) initAgentPage(target);
  });
}
