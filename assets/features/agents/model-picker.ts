function initPicker(picker: HTMLElement) {
  if (picker.dataset.modelPickerBound === "true") return;
  const input = picker.parentElement?.querySelector<HTMLInputElement>('input[name="model_selection"]');
  const variantInput = picker.parentElement?.querySelector<HTMLInputElement>('input[name="model_variant"]');
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
  const variantArea = picker.querySelector<HTMLElement>("[data-model-picker-variant-area]");
  const variantPanels = Array.from(picker.querySelectorAll<HTMLElement>("[data-model-picker-variant-panel]"));
  const unavailableVariantControl = picker.querySelector<HTMLElement>("[data-model-picker-variant-unavailable]");
  const variantWarning = picker.querySelector<HTMLElement>("[data-model-picker-variant-warning]");
  let draftModel = input.value;
  let draftVariant = variantInput?.value ?? "";
  let activeProvider = "__none__";
  const selectedOption = (value: string) => options.find((option) => option.dataset.value === value) ?? options[0];
  const activeVariantPanel = (model: string) => variantPanels.find((panel) => panel.dataset.modelValue === model);
  const variantIsAvailable = (model: string, variant: string) => {
    if (!variant) return true;
    return Array.from(activeVariantPanel(model)?.querySelectorAll<HTMLOptionElement>("option") ?? []).some((option) => option.value === variant);
  };
  const unavailableVariantMessage = (model: string, variant: string) => variant && !variantIsAvailable(model, variant)
    ? `Thinking mode “${variant}” is no longer available for this model. Choose default or a current mode.`
    : "";
  const updateSelected = (option: HTMLElement, variant: string) => {
    const modelLabel = option.dataset.label ?? "None selected";
    label.textContent = variant ? `${modelLabel} - ${variant}` : modelLabel;
    logo.src = option.dataset.logoUrl ?? "";
    logo.classList.toggle("hidden", !option.dataset.logoUrl);
    defaultBadge.classList.toggle("hidden", Boolean(option.dataset.logoUrl));
  };
  const updateVariantControls = (model: string, variant: string) => {
    const panel = activeVariantPanel(model);
    const warning = unavailableVariantMessage(model, variant);
    variantArea?.classList.remove("hidden");
    variantPanels.forEach((entry) => entry.classList.toggle("hidden", entry !== panel));
    unavailableVariantControl?.classList.toggle("hidden", Boolean(panel));
    const unavailableSelect = unavailableVariantControl?.querySelector<HTMLSelectElement>("[data-model-picker-variant-unavailable-select]");
    if (unavailableSelect) unavailableSelect.disabled = !warning;
    if (panel) {
      const select = panel.querySelector<HTMLSelectElement>("[data-model-picker-variant-select]");
      if (select) select.value = variantIsAvailable(model, variant) ? variant : "";
    }
    if (variantWarning) {
      variantWarning.textContent = warning;
      variantWarning.classList.toggle("hidden", !warning);
    }
  };
  const currentModel = () => draftModel;
  const currentVariant = () => draftVariant;
  const selectionIsValid = () => !unavailableVariantMessage(currentModel(), currentVariant());
  const notifyChange = (element: HTMLInputElement) => element.dispatchEvent(new Event("change", { bubbles: true }));
  const filter = () => {
    const query = search?.value.trim().toLowerCase() ?? "";
    const matchingProviders = new Set<string>();
    options.forEach((option) => {
      const matches = !query || (option.dataset.searchText ?? option.dataset.label ?? "").toLowerCase().includes(query);
      if (matches) matchingProviders.add(option.dataset.providerId ?? "__none__");
    });
    if (query && !matchingProviders.has(activeProvider)) {
      activeProvider = providers.find((provider) => matchingProviders.has(provider.dataset.providerId ?? "__none__"))?.dataset.providerId ?? "__none__";
    }
    options.forEach((option) => {
      const matches = !query || (option.dataset.searchText ?? option.dataset.label ?? "").toLowerCase().includes(query);
      const visible = matches && option.dataset.providerId === activeProvider;
      option.classList.toggle("hidden", !visible);
      option.classList.toggle("bg-zinc-900", option.dataset.value === draftModel);
      option.classList.toggle("text-white", option.dataset.value === draftModel);
    });
    providers.forEach((provider) => {
      const visible = !query || (provider.dataset.searchText ?? "").toLowerCase().includes(query) || matchingProviders.has(provider.dataset.providerId ?? "__none__");
      provider.classList.toggle("hidden", !visible);
      provider.classList.toggle("bg-zinc-900", provider.dataset.providerId === activeProvider);
      provider.classList.toggle("text-white", provider.dataset.providerId === activeProvider);
    });
  };
  const close = () => { modal?.classList.add("hidden"); document.body.classList.remove("overflow-hidden"); };
  picker.querySelector<HTMLElement>("[data-model-picker-open]")?.addEventListener("click", () => {
    draftModel = input.value;
    draftVariant = variantInput?.value ?? "";
    activeProvider = selectedOption(draftModel)?.dataset.providerId ?? "__none__";
    if (search) search.value = "";
    updateVariantControls(draftModel, draftVariant);
    filter();
    modal?.classList.remove("hidden");
    document.body.classList.add("overflow-hidden");
    search?.focus();
  });
  picker.querySelectorAll<HTMLElement>("[data-model-picker-close], [data-model-picker-cancel]").forEach((button) => button.addEventListener("click", close));
  search?.addEventListener("input", filter);
  providers.forEach((provider) => provider.addEventListener("click", () => {
    activeProvider = provider.dataset.providerId ?? "__none__";
    if (activeProvider === "__none__") {
      draftModel = "";
      draftVariant = "";
      updateVariantControls(draftModel, draftVariant);
    }
    filter();
  }));
  options.forEach((option) => option.addEventListener("click", () => {
    const nextModel = option.dataset.value ?? "";
    if (nextModel !== draftModel) draftVariant = "";
    draftModel = nextModel;
    activeProvider = option.dataset.providerId ?? "__none__";
    updateVariantControls(draftModel, draftVariant);
    filter();
  }));
  variantPanels.forEach((panel) => panel.querySelector<HTMLSelectElement>("[data-model-picker-variant-select]")?.addEventListener("change", (event) => {
    const select = event.currentTarget;
    if (!(select instanceof HTMLSelectElement)) return;
    draftVariant = select.value;
    updateVariantControls(draftModel, draftVariant);
  }));
  unavailableVariantControl?.querySelector<HTMLSelectElement>("[data-model-picker-variant-unavailable-select]")?.addEventListener("change", () => {
    draftVariant = "";
    updateVariantControls(draftModel, draftVariant);
  });
  picker.querySelector<HTMLElement>("[data-model-picker-save]")?.addEventListener("click", () => {
    if (!selectionIsValid()) {
      updateVariantControls(draftModel, draftVariant);
      activeVariantPanel(draftModel)?.querySelector<HTMLSelectElement>("[data-model-picker-variant-select]")?.focus();
      return;
    }
    input.value = draftModel;
    notifyChange(input);
    if (variantInput) {
      variantInput.value = draftVariant;
      notifyChange(variantInput);
    }
    updateSelected(selectedOption(draftModel), draftVariant);
    close();
    if (picker.dataset.modelPickerSubmitOnSave === "true") form?.requestSubmit();
  });
  form?.addEventListener("submit", (event) => {
    if (selectionIsValid()) return;
    event.preventDefault();
    updateVariantControls(currentModel(), currentVariant());
    activeVariantPanel(currentModel())?.querySelector<HTMLSelectElement>("[data-model-picker-variant-select]")?.focus();
  });
  updateVariantControls(input.value, variantInput?.value ?? "");
  updateSelected(selectedOption(input.value), variantInput?.value ?? "");
}

export function initModelPickers(root: ParentNode = document) {
  if (root instanceof HTMLElement && root.matches("[data-model-picker]")) initPicker(root);
  root.querySelectorAll<HTMLElement>("[data-model-picker]").forEach(initPicker);
}

export function installModelPickerLifecycle() {
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    const modals = document.querySelectorAll<HTMLElement>("[data-model-picker-modal]:not(.hidden)");
    modals.forEach((modal) => modal.classList.add("hidden"));
    if (modals.length) document.body.classList.remove("overflow-hidden");
  });
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
