async function copy(value: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
      return true;
    }
  } catch {
    // Fall through to the compatibility path below.
  }

  {
    const textarea = document.createElement("textarea");
    textarea.value = value;
    textarea.readOnly = true;
    textarea.style.cssText = "position:fixed;opacity:0";
    document.body.append(textarea);
    textarea.select();
    try {
      return document.execCommand("copy");
    } finally {
      textarea.remove();
    }
  }
}

export function installCopyButtons() {
  document.addEventListener("click", (event) => {
    const button = (event.target as Element | null)?.closest<HTMLButtonElement>("[data-copy-button]");
    const value = button?.dataset.copyValue;
    if (!button || !value) {
      return;
    }
    const label = button.dataset.copyLabel ?? button.ariaLabel ?? "Copy";
    button.dataset.copyLabel = label;
    void copy(value).then((copied) => {
      button.classList.remove("text-zinc-400", "text-emerald-300", "text-red-300");
      button.classList.add(copied ? "text-emerald-300" : "text-red-300");
      button.ariaLabel = copied ? "Copied" : "Copy failed";
      button.title = button.ariaLabel;
      window.setTimeout(() => {
        button.classList.remove("text-emerald-300", "text-red-300");
        button.classList.add("text-zinc-400");
        button.ariaLabel = label;
        button.title = label;
      }, 1500);
    });
  });
}
