export function csrfToken(): string | undefined {
  return document.cookie
    .split("; ")
    .find((cookie) => cookie.startsWith("vt_csrf="))
    ?.split("=", 2)[1];
}

function seedForm(form: HTMLFormElement, token: string) {
  if (form.method.toLowerCase() === "get") {
    return;
  }
  let input = form.querySelector<HTMLInputElement>('input[name="csrf_token"]');
  if (!input) {
    input = document.createElement("input");
    input.type = "hidden";
    input.name = "csrf_token";
    form.append(input);
  }
  input.value = token;
}

export function seedCsrfTokens(root: ParentNode = document) {
  const token = csrfToken();
  if (!token) {
    return;
  }
  if (root instanceof HTMLFormElement) {
    seedForm(root, token);
  }
  root.querySelectorAll<HTMLFormElement>("form[method]").forEach((form) => seedForm(form, token));
}

export function installCsrf() {
  document.addEventListener("htmx:configRequest", (event) => {
    const token = csrfToken();
    if (token) {
      (event as CustomEvent<{ headers: Record<string, string> }>).detail.headers["X-CSRF-Token"] = token;
    }
  });
  document.addEventListener(
    "submit",
    (event) => {
      const token = csrfToken();
      if (token && event.target instanceof HTMLFormElement) {
        seedForm(event.target, token);
      }
    },
    true,
  );
}
