import * as htmx from "htmx.org";
(window as any).htmx = htmx;
import "htmx-ext-sse";

import { render } from "timeago.js";

function renderTimeago(root: ParentNode = document) {
  const nodes = root.querySelectorAll("time.timeago");
  if (nodes.length > 0) {
    render(nodes);
  }
}

function init() {
  renderTimeago();

  if (typeof MutationObserver === "undefined") {
    return;
  }

  const observer = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      for (const node of mutation.addedNodes) {
        if (!(node instanceof Element)) {
          continue;
        }
        if (node.matches("time.timeago")) {
          render([node]);
        }
        renderTimeago(node);
      }
    }
  });

  observer.observe(document.documentElement, {
    childList: true,
    subtree: true,
  });
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", init);
} else {
  init();
}
