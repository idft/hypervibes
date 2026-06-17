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

let prevFormatted: string | null = null;

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

function animateBalanceRoll(container: HTMLElement) {
  const digits = container.querySelectorAll<HTMLElement>(".balance-digit");
  if (digits.length === 0) return;

  const current = Array.from(digits, (d) => d.getAttribute("data-digit") ?? "").join("");

  if (prevFormatted != null && prevFormatted !== current) {
    const rawPrev = parseFloat(prevFormatted);
    const rawNext = parseFloat(current);
    const up = !isNaN(rawPrev) && !isNaN(rawNext) && rawNext > rawPrev;
    const keyframes = up ? rollUpKeyframes : rollDownKeyframes;

    let firstChanged = -1;
    for (let i = 0; i < digits.length; i++) {
      if (i < prevFormatted.length && prevFormatted[i] !== current[i]) {
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

  prevFormatted = current;
}

function init() {
  renderTimeago();

  const initial = document.querySelector(".balance-roll") as HTMLElement | null;
  if (initial) {
    const digits = initial.querySelectorAll<HTMLElement>(".balance-digit");
    if (digits.length > 0) {
      prevFormatted = Array.from(digits, (d) => d.getAttribute("data-digit") ?? "").join("");
    }
  }

  document.addEventListener("htmx:sseMessage", (e: Event) => {
    const detail = (e as CustomEvent).detail;
    if (detail?.type === "balance") {
      setTimeout(() => {
        const roll = document.querySelector(".balance-roll") as HTMLElement | null;
        if (roll) animateBalanceRoll(roll);
      }, 50);
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
