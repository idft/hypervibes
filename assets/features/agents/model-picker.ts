function initPicker(picker: HTMLElement) {
  if (picker.dataset.modelPickerBound === "true") return;
  const input = picker.parentElement?.querySelector<HTMLInputElement>('input[name="model_selection"]');
  const form = input?.closest<HTMLFormElement>("form");
  const label = picker.querySelector<HTMLElement>("[data-model-picker-label]");
  const logo = picker.querySelector<HTMLImageElement>("[data-model-picker-logo]");
  const defaultBadge = picker.querySelector<HTMLElement>("[data-model-picker-default-badge]");
  if (!input || !label || !logo || !defaultBadge) return;
  picker.dataset.modelPickerBound = "true";
  const modal = picker.querySelector<HTMLElement>("[data-model-picker-modal]");
  const search = picker.querySelector<HTMLInputElement>("[data-model-picker-search]");
  const options = Array.from(picker.querySelectorAll<HTMLElement>("[data-model-picker-option]"));
  const providers = Array.from(picker.querySelectorAll<HTMLElement>("[data-model-picker-provider]"));
  const isModal = picker.dataset.modelPickerMode === "modal";
  let draft = input.value;
  let activeProvider = "__none__";
  const selectedOption = (value: string) => options.find((option) => option.dataset.value === value) ?? options[0];
  const updateSelected = (option: HTMLElement) => { label.textContent = option.dataset.label ?? "None selected"; logo.src = option.dataset.logoUrl ?? ""; logo.classList.toggle("hidden", !option.dataset.logoUrl); defaultBadge.classList.toggle("hidden", Boolean(option.dataset.logoUrl)); };
  const filter = () => {
    const query = search?.value.trim().toLowerCase() ?? "";
    const matchingProviders = new Set<string>();
    options.forEach((option) => {
      const matches = !query || (option.dataset.searchText ?? option.dataset.label ?? "").toLowerCase().includes(query);
      if (matches) matchingProviders.add(option.dataset.providerId ?? "__none__");
    });
    if (isModal && query && !matchingProviders.has(activeProvider)) {
      activeProvider = providers.find((provider) => matchingProviders.has(provider.dataset.providerId ?? "__none__"))?.dataset.providerId ?? "__none__";
    }
    options.forEach((option) => {
      const matches = !query || (option.dataset.searchText ?? option.dataset.label ?? "").toLowerCase().includes(query);
      const visible = matches && (!isModal || option.dataset.providerId === activeProvider);
      option.classList.toggle("hidden", !visible);
      option.classList.toggle("bg-zinc-900", option.dataset.value === draft);
      option.classList.toggle("text-white", option.dataset.value === draft);
    });
    if (isModal) providers.forEach((provider) => {
      const visible = !query || (provider.dataset.searchText ?? "").toLowerCase().includes(query) || matchingProviders.has(provider.dataset.providerId ?? "__none__");
      provider.classList.toggle("hidden", !visible);
      provider.classList.toggle("bg-zinc-900", provider.dataset.providerId === activeProvider);
      provider.classList.toggle("text-white", provider.dataset.providerId === activeProvider);
    });
  };
  const close = () => { modal?.classList.add("hidden"); document.body.classList.remove("overflow-hidden"); };
  picker.querySelector<HTMLElement>("[data-model-picker-open]")?.addEventListener("click", () => { draft = input.value; activeProvider = selectedOption(draft)?.dataset.providerId ?? "__none__"; if (search) search.value = ""; filter(); modal?.classList.remove("hidden"); document.body.classList.add("overflow-hidden"); search?.focus(); });
  picker.querySelectorAll<HTMLElement>("[data-model-picker-close], [data-model-picker-cancel]").forEach((button) => button.addEventListener("click", close));
  search?.addEventListener("input", filter);
  providers.forEach((provider) => provider.addEventListener("click", () => { activeProvider = provider.dataset.providerId ?? "__none__"; if (activeProvider === "__none__") draft = ""; filter(); }));
  options.forEach((option) => option.addEventListener("click", () => { if (isModal) { draft = option.dataset.value ?? ""; activeProvider = option.dataset.providerId ?? "__none__"; filter(); return; } input.value = option.dataset.value ?? ""; input.dispatchEvent(new Event("change", { bubbles: true })); updateSelected(option); if (search) search.value = ""; options.forEach((entry) => entry.classList.remove("hidden")); picker.removeAttribute("open"); if (picker.dataset.modelPickerAutoSubmit === "true") form?.requestSubmit(); }));
  picker.querySelector<HTMLElement>("[data-model-picker-save]")?.addEventListener("click", () => { input.value = draft; input.dispatchEvent(new Event("change", { bubbles: true })); const option = selectedOption(draft); if (option) updateSelected(option); close(); form?.requestSubmit(); });
  const option = selectedOption(input.value); if (option) updateSelected(option);
}

export function initModelPickers(root: ParentNode = document) {
  if (root instanceof HTMLElement && root.matches("[data-model-picker]")) initPicker(root);
  root.querySelectorAll<HTMLElement>("[data-model-picker]").forEach(initPicker);
}

export function installModelPickerLifecycle() {
  document.addEventListener("keydown", (event) => { if (event.key === "Escape") document.querySelectorAll<HTMLElement>("[data-model-picker-modal]:not(.hidden)").forEach((modal) => modal.classList.add("hidden")); });
  document.addEventListener("htmx:load", (event) => {
    const target = (event as CustomEvent<{ elt?: unknown }>).detail.elt;
    if (target instanceof Element) initModelPickers(target);
  });
  document.addEventListener("htmx:afterSwap", (event) => {
    const target = (event as CustomEvent<{ target?: unknown }>).detail.target;
    if (!(target instanceof Element)) return;
    initModelPickers(target);
    // Outer swaps can fire before the newly selected subtree is connected.
    // Retry on the next frame; the bound marker keeps this idempotent.
    window.requestAnimationFrame(() => initModelPickers(target));
    const picker = target.querySelector<HTMLElement>("[data-model-picker-lazy-result] [data-model-picker]");
    picker?.querySelector<HTMLElement>("[data-model-picker-open]")?.click();
  });
}
