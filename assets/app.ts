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

function init() {
  renderTimeago();

  document.querySelectorAll<HTMLElement>(".number-roll").forEach(seedNumberRoll);

  document.addEventListener("htmx:sseMessage", (e: Event) => {
    const detail = (e as CustomEvent).detail;
    if (detail?.type === "balance" || detail?.type === "positions") {
      setTimeout(() => {
        document.querySelectorAll<HTMLElement>(".number-roll").forEach(animateNumberRoll);
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
          render([node as HTMLElement]);
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
