import * as htmx from "htmx.org";
(window as unknown as { htmx: typeof htmx }).htmx = htmx;
import "htmx-ext-sse";

import { render } from "timeago.js";

function renderTimeago(root: ParentNode = document) {
  const nodes = root.querySelectorAll("time.timeago");
  if (nodes.length > 0) {
    render(nodes);
  }
}

interface NumberState {
  raw: string;
  formatted: string;
}

const previousValues = new Map<string, NumberState>();

const rollUpKeyframes: Keyframe[] = [
  { transform: "translateY(-0.4em)", opacity: "0", color: "#4ade80" },
  { transform: "translateY(0)", opacity: "1", color: "#4ade80", offset: 0.85 },
  { transform: "translateY(0)", opacity: "1", color: "inherit" },
];

const rollDownKeyframes: Keyframe[] = [
  { transform: "translateY(0.4em)", opacity: "0", color: "#f87171" },
  { transform: "translateY(0)", opacity: "1", color: "#f87171", offset: 0.85 },
  { transform: "translateY(0)", opacity: "1", color: "inherit" },
];

const rollTiming: KeyframeAnimationOptions = {
  duration: 1500,
  easing: "cubic-bezier(0.22, 0.61, 0.36, 1)",
  fill: "forwards",
};

function readFormattedDigits(container: HTMLElement): string {
  const digits = container.querySelectorAll<HTMLElement>(".number-digit");
  return Array.from(digits, (d) => d.getAttribute("data-digit") ?? "").join("");
}

function animateNumberRoll(container: HTMLElement) {
  const key = container.getAttribute("data-animate-key");
  if (!key) return;

  const digits = container.querySelectorAll<HTMLElement>(".number-digit");
  if (digits.length === 0) return;

  const rawValue = container.getAttribute("data-raw-value") ?? "";
  const formattedValue = readFormattedDigits(container);

  const prev = previousValues.get(key);

  if (prev && prev.formatted !== formattedValue) {
    const rawPrev = parseFloat(prev.raw);
    const rawNext = parseFloat(rawValue);
    const up = !isNaN(rawPrev) && !isNaN(rawNext) && rawNext > rawPrev;
    const keyframes = up ? rollUpKeyframes : rollDownKeyframes;

    let firstChanged = -1;
    for (let i = 0; i < digits.length; i++) {
      const digit = digits[i].getAttribute("data-digit") ?? "";
      if (i >= prev.formatted.length || prev.formatted[i] !== digit) {
        firstChanged = i;
        break;
      }
    }

    if (firstChanged >= 0) {
      let stagger = 0;
      for (let i = firstChanged; i < digits.length; i++) {
        digits[i].animate(keyframes, { ...rollTiming, delay: stagger });
        stagger += 60;
      }
    }
  }

  previousValues.set(key, { raw: rawValue, formatted: formattedValue });
}

function seedNumberRoll(container: HTMLElement) {
  const key = container.getAttribute("data-animate-key");
  if (!key) return;
  const rawValue = container.getAttribute("data-raw-value") ?? "";
  const formattedValue = readFormattedDigits(container);
  previousValues.set(key, { raw: rawValue, formatted: formattedValue });
}

function setActiveMemoryTimelineItem(activeItem: HTMLElement) {
  document
    .querySelectorAll<HTMLElement>("[data-memory-timeline-item]")
    .forEach((item) => {
      item.setAttribute("aria-pressed", item === activeItem ? "true" : "false");
    });
}

function loadMemoryTimelineItem(item: HTMLElement) {
  const url = item.getAttribute("hx-get");
  const target = item.getAttribute("hx-target");
  if (!url || !target) {
    return;
  }

  const targetElement = document.querySelector(target);
  if (!(targetElement instanceof HTMLElement)) {
    return;
  }

  setActiveMemoryTimelineItem(item);
  item.setAttribute("aria-busy", "true");

  void fetch(url, {
    headers: {
      "HX-Request": "true",
    },
  })
    .then(async (response) => {
      if (!response.ok) {
        throw new Error(`Failed to load memory detail: ${response.status}`);
      }
      const html = await response.text();
      targetElement.outerHTML = html;
    })
    .catch((error) => {
      console.error(error);
    })
    .finally(() => {
      item.removeAttribute("aria-busy");
    });
}

function initMemoryTimelineDragScroll() {
  document
    .querySelectorAll<HTMLElement>(".memory-timeline-scroll")
    .forEach((container) => {
      if (container.dataset.dragScrollBound === "true") {
        return;
      }
      container.dataset.dragScrollBound = "true";

      let mouseDown = false;
      let startX = 0;
      let startScrollLeft = 0;
      let dragged = false;
      let pressedItem: HTMLElement | null = null;
      let suppressClickFor: HTMLElement | null = null;
      let cleanupListeners: (() => void) | null = null;

      const stopDragging = () => {
        mouseDown = false;
        pressedItem = null;
        cleanupListeners?.();
        cleanupListeners = null;
        container.dataset.dragging = "false";
      };

      container.addEventListener("mousedown", (event: MouseEvent) => {
        if (event.button !== 0) {
          return;
        }

        mouseDown = true;
        startX = event.clientX;
        startScrollLeft = container.scrollLeft;
        dragged = false;
        pressedItem = (event.target as Element | null)?.closest<HTMLElement>(
          "[data-memory-timeline-item]",
        ) ?? null;
        container.dataset.dragging = "false";

        const handleMouseMove = (moveEvent: MouseEvent) => {
          if (!mouseDown) {
            return;
          }

          const deltaX = moveEvent.clientX - startX;
          if (!dragged && Math.abs(deltaX) > 6) {
            dragged = true;
            container.dataset.dragging = "true";
          }
          if (!dragged) {
            return;
          }

          moveEvent.preventDefault();
          container.scrollLeft = startScrollLeft - deltaX;
        };

        const handleMouseUp = () => {
          if (!mouseDown) {
            return;
          }

          if (!dragged && pressedItem) {
            suppressClickFor = pressedItem;
            loadMemoryTimelineItem(pressedItem);
            window.setTimeout(() => {
              suppressClickFor = null;
            }, 0);
          }
          stopDragging();
        };

        const handleWindowBlur = () => {
          stopDragging();
        };

        window.addEventListener("mousemove", handleMouseMove, { passive: false });
        window.addEventListener("mouseup", handleMouseUp);
        window.addEventListener("blur", handleWindowBlur);
        cleanupListeners = () => {
          window.removeEventListener("mousemove", handleMouseMove);
          window.removeEventListener("mouseup", handleMouseUp);
          window.removeEventListener("blur", handleWindowBlur);
        };
      });

      container.addEventListener(
        "click",
        (event) => {
          const item = (event.target as Element | null)?.closest<HTMLElement>(
            "[data-memory-timeline-item]",
          );
          if (!item) {
            return;
          }

          if (suppressClickFor === item) {
            event.preventDefault();
            event.stopPropagation();
            suppressClickFor = null;
            return;
          }

          if (dragged) {
            event.preventDefault();
            event.stopPropagation();
            dragged = false;
            return;
          }

          event.preventDefault();
          event.stopPropagation();
          loadMemoryTimelineItem(item);
        },
        true,
      );
    });
}

function init() {
  renderTimeago();
  initMemoryTimelineDragScroll();

  document.querySelectorAll<HTMLElement>(".number-roll").forEach(seedNumberRoll);

  document.addEventListener("htmx:sseMessage", (e: Event) => {
    const detail = (e as CustomEvent).detail;
    if (detail?.type === "balance" || detail?.type === "positions") {
      setTimeout(() => {
        document.querySelectorAll<HTMLElement>(".number-roll").forEach(animateNumberRoll);
      }, 50);
    }
  });

  document.addEventListener("htmx:afterRequest", (e: Event) => {
    const detail = (e as CustomEvent).detail;
    if (!detail?.successful) {
      return;
    }

    const elt = detail.elt;
    if (elt instanceof HTMLElement && elt.matches("[data-memory-timeline-item]")) {
      setActiveMemoryTimelineItem(elt);
    }
  });

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
          render([node as HTMLElement]);
        }
        renderTimeago(node);
        if (node instanceof HTMLElement) {
          if (node.matches(".memory-timeline-scroll")) {
            initMemoryTimelineDragScroll();
          }
          if (node.querySelector(".memory-timeline-scroll")) {
            initMemoryTimelineDragScroll();
          }
        }
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
