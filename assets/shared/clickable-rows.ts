const INTERACTIVE_SELECTOR = "a, button, input, select, textarea, label, form, summary";

export function initClickableRows(root: ParentNode = document) {
  root.querySelectorAll<HTMLElement>("[data-row-href]").forEach((row) => {
    if (row.dataset.rowHrefBound === "true") {
      return;
    }
    row.dataset.rowHrefBound = "true";
    const navigate = () => {
      const href = row.dataset.rowHref;
      if (href) {
        window.location.assign(href);
      }
    };
    row.addEventListener("click", (event) => {
      if ((event.target as Element | null)?.closest(INTERACTIVE_SELECTOR)) {
        return;
      }
      navigate();
    });
    row.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        navigate();
      }
    });
  });
}
