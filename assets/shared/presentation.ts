import { render } from "timeago.js";

type LocalDateTimeFormat = "date" | "time" | "datetime";
type NumberState = { raw: string; formatted: string };

const previousValues = new Map<string, NumberState>();
const rollTiming: KeyframeAnimationOptions = {
  duration: 1500,
  easing: "cubic-bezier(0.22, 0.61, 0.36, 1)",
  fill: "forwards",
};

export function renderTimeago(root: ParentNode = document) {
  const nodes = root.querySelectorAll<HTMLElement>("time.timeago");
  if (nodes.length > 0) {
    render(nodes);
    nodes.forEach((node) => node.classList.add("timeago-ready"));
  }
}

function formatLocalDateTime(date: Date, format: LocalDateTimeFormat): string {
  const options: Intl.DateTimeFormatOptions =
    format === "date"
      ? { year: "numeric", month: "short", day: "numeric" }
      : format === "time"
        ? { hour: "numeric", minute: "2-digit" }
        : { year: "numeric", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" };
  return new Intl.DateTimeFormat(undefined, options).format(date);
}

export function renderLocalDateTimes(root: ParentNode = document) {
  root.querySelectorAll<HTMLTimeElement>("time.local-datetime").forEach((node) => {
    const value = node.dateTime;
    const date = new Date(value);
    if (!value || Number.isNaN(date.getTime())) {
      return;
    }
    const format = (node.dataset.localFormat as LocalDateTimeFormat | undefined) ?? "datetime";
    node.textContent = formatLocalDateTime(date, format);
  });
}

function readDigits(container: HTMLElement) {
  return Array.from(container.querySelectorAll<HTMLElement>(".number-digit"), (digit) => digit.dataset.digit ?? "").join("");
}

export function seedNumberRolls(root: ParentNode = document) {
  root.querySelectorAll<HTMLElement>(".number-roll").forEach((container) => {
    const key = container.dataset.animateKey;
    if (key) {
      previousValues.set(key, { raw: container.dataset.rawValue ?? "", formatted: readDigits(container) });
    }
  });
}

export function animateNumberRolls() {
  document.querySelectorAll<HTMLElement>(".number-roll").forEach((container) => {
    const key = container.dataset.animateKey;
    if (!key) {
      return;
    }
    const formatted = readDigits(container);
    const previous = previousValues.get(key);
    const digits = container.querySelectorAll<HTMLElement>(".number-digit");
    if (previous && previous.formatted !== formatted) {
      const up = Number(container.dataset.rawValue) > Number(previous.raw);
      const keyframes: Keyframe[] = up
        ? [{ transform: "translateY(-0.4em)", opacity: "0", color: "#4ade80" }, { transform: "translateY(0)", opacity: "1", color: "inherit" }]
        : [{ transform: "translateY(0.4em)", opacity: "0", color: "#f87171" }, { transform: "translateY(0)", opacity: "1", color: "inherit" }];
      let delay = 0;
      digits.forEach((digit) => {
        digit.animate(keyframes, { ...rollTiming, delay });
        delay += 60;
      });
    }
    previousValues.set(key, { raw: container.dataset.rawValue ?? "", formatted });
  });
}

function formatElapsedDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  if (total < 60) return `${total}s`;
  if (total < 3600) return `${Math.floor(total / 60)}m${total % 60 === 0 ? "" : ` ${total % 60}s`}`;
  return `${Math.floor(total / 3600)}h${total % 3600 === 0 ? "" : ` ${Math.floor((total % 3600) / 60)}m`}`;
}

export function tickRunningDurations() {
  const now = Date.now();
  document.querySelectorAll<HTMLElement>("[data-running-duration]").forEach((node) => {
    const startedAt = node.dataset.startedAt;
    if (startedAt) {
      const started = new Date(startedAt).getTime();
      if (!Number.isNaN(started)) node.textContent = formatElapsedDuration((now - started) / 1000);
    }
  });
}

export function startRunningDurationTicker() {
  tickRunningDurations();
  window.setInterval(tickRunningDurations, 1000);
}
