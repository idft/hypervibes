function initSingletonRoleForm(form: HTMLFormElement) {
  if (form.dataset.singletonRoleFormBound === "true") return;
  const model = form.querySelector<HTMLInputElement>('input[name="model_selection"]');
  const prompt = form.querySelector<HTMLTextAreaElement>('textarea[name="prompt"]');
  const submit = form.querySelector<HTMLButtonElement>("[data-singleton-role-submit]");
  if (!model || !prompt || !submit) return;

  form.dataset.singletonRoleFormBound = "true";
  const sync = () => { submit.disabled = !model.value.trim() || !prompt.value.trim(); };
  form.addEventListener("input", sync);
  form.addEventListener("change", sync);
  sync();
}

export function initSingletonRoleForms(root: ParentNode = document) {
  root.querySelectorAll<HTMLFormElement>("form[data-singleton-role-form]").forEach(initSingletonRoleForm);
}
