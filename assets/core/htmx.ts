import htmx from "htmx.org";
import "htmx-ext-sse";

(window as Window & { htmx?: typeof htmx }).htmx = htmx;

type ClosableEventSource = { close: () => void };

const sseSources = new Map<Element, Set<ClosableEventSource>>();

export function preventSseReconnectOnRemovedElement(element: Element) {
  // htmx-ext-sse can schedule a reconnect after the source is closed. Removing
  // its URL prevents that timer from reopening a stream on detached content.
  element.removeAttribute("sse-connect");
}

function closeSseSources(element: Element) {
  for (const source of sseSources.get(element) ?? []) source.close();
  sseSources.delete(element);
}

document.addEventListener("htmx:sseOpen", (event) => {
  const element = event.target;
  const source = (event as CustomEvent<{ source?: ClosableEventSource }>).detail.source;
  if (!(element instanceof Element) || !source || typeof source.close !== "function") return;
  const sources = sseSources.get(element) ?? new Set<ClosableEventSource>();
  sources.add(source);
  sseSources.set(element, sources);
});

document.addEventListener("htmx:sseClose", (event) => {
  const element = event.target;
  const source = (event as CustomEvent<{ source?: ClosableEventSource }>).detail.source;
  if (!(element instanceof Element) || !source) return;
  const sources = sseSources.get(element);
  sources?.delete(source);
  if (sources?.size === 0) sseSources.delete(element);
});

document.addEventListener("htmx:beforeCleanupElement", (event) => {
  const element = (event as CustomEvent<{ elt?: unknown }>).detail.elt;
  if (!(element instanceof Element)) return;

  const streamElements = [element, ...element.querySelectorAll("[sse-connect]")];
  for (const streamElement of streamElements) {
    if (streamElement.hasAttribute("sse-connect")) {
      preventSseReconnectOnRemovedElement(streamElement);
    }
    closeSseSources(streamElement);
  }
});

document.addEventListener("htmx:beforeRequest", () => {
  for (const element of sseSources.keys()) {
    if (!element.isConnected) {
      preventSseReconnectOnRemovedElement(element);
      closeSseSources(element);
    }
  }
});

export { htmx };
