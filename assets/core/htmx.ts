import htmx from "htmx.org";
import "htmx-ext-sse";

(window as Window & { htmx?: typeof htmx }).htmx = htmx;

export function preventSseReconnectOnRemovedElement(element: Element) {
  // htmx-ext-sse can schedule a reconnect after the source is closed. Removing
  // its URL prevents that timer from reopening a stream on detached content.
  element.removeAttribute("sse-connect");
}

document.addEventListener("htmx:beforeCleanupElement", (event) => {
  const element = (event as CustomEvent<{ elt?: unknown }>).detail.elt;
  if (element instanceof Element && element.hasAttribute("sse-connect")) {
    preventSseReconnectOnRemovedElement(element);
  }
});

export { htmx };
