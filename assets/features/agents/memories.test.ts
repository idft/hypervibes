import { afterEach, describe, expect, it, vi } from "vitest";

import { initMemoryTimelines, restoreSelectedMemoryItems } from "./memories";

afterEach(() => {
  document.body.replaceChildren();
});

describe("memory timeline selection", () => {
  it("marks the detail memory as selected in the timeline", () => {
    document.body.innerHTML = `
      <section data-agent-memories data-selected-memory-id="stale">
        <button data-memory-timeline-item data-memory-id="first" aria-pressed="true"></button>
        <button data-memory-timeline-item data-memory-id="second" aria-pressed="false"></button>
        <div id="memory-detail" data-memory-detail-id="second"></div>
      </section>
    `;

    restoreSelectedMemoryItems();

    const items = document.querySelectorAll<HTMLElement>("[data-memory-timeline-item]");
    expect(items[0].getAttribute("aria-pressed")).toBe("false");
    expect(items[1].getAttribute("aria-pressed")).toBe("true");
  });
});

describe("local date filtering", () => {
  it("reveals custom dates and submits the current browser time zone", () => {
    const today = new Date();
    const localToday = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
    document.body.innerHTML = `
      <section data-agent-memories>
        <button type="button" data-memory-custom-toggle aria-expanded="false">Custom</button>
        <form data-memory-date-form class="hidden">
          <input type="hidden" name="range" value="custom">
          <input type="date" name="start" value="${localToday}">
          <input type="date" name="end" value="${localToday}">
          <input type="hidden" name="tz" data-memory-timezone value="UTC">
          <button type="button" data-memory-custom-cancel>Cancel</button>
        </form>
      </section>
    `;
    const form = document.querySelector<HTMLFormElement>("form")!;
    initMemoryTimelines();
    document.querySelector<HTMLButtonElement>("[data-memory-custom-toggle]")!.click();
    expect(form.classList.contains("hidden")).toBe(false);
    form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    expect(new FormData(form).get("tz")).toBe(Intl.DateTimeFormat().resolvedOptions().timeZone ?? "UTC");
    expect(new FormData(form).get("start")).toBe(localToday);
    expect(new FormData(form).get("end")).toBe(localToday);
  });

  it("cancels unapplied custom dates and returns focus to Custom", () => {
    document.body.innerHTML = `
      <section data-agent-memories>
        <button type="button" data-memory-custom-toggle aria-expanded="false">Custom</button>
        <form data-memory-date-form class="hidden">
          <input type="date" name="start" value="2025-01-01">
          <input type="date" name="end" value="2025-01-02">
          <button type="button" data-memory-custom-cancel>Cancel</button>
        </form>
      </section>`;
    const toggle = document.querySelector<HTMLButtonElement>("[data-memory-custom-toggle]")!;
    const form = document.querySelector<HTMLFormElement>("form")!;
    initMemoryTimelines();
    toggle.click();
    form.querySelector<HTMLInputElement>('input[name="start"]')!.value = "2025-02-01";
    form.querySelector<HTMLButtonElement>("[data-memory-custom-cancel]")!.click();
    expect(form.classList.contains("hidden")).toBe(true);
    expect(form.querySelector<HTMLInputElement>('input[name="start"]')!.value).toBe("2025-01-01");
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(toggle);
  });

  it("limits both calendars to today in the browser's local time zone", () => {
    document.body.innerHTML = `<section data-agent-memories><form data-memory-date-form><input type="date" name="start"><input type="date" name="end"></form></section>`;
    initMemoryTimelines();
    const today = new Date();
    const localToday = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
    const tomorrow = new Date(today);
    tomorrow.setDate(tomorrow.getDate() + 1);
    const nextDay = `${tomorrow.getFullYear()}-${String(tomorrow.getMonth() + 1).padStart(2, "0")}-${String(tomorrow.getDate()).padStart(2, "0")}`;
    document.querySelectorAll<HTMLInputElement>('input[type="date"]').forEach((input) => {
      expect(input.max).toBe(localToday);
      input.value = nextDay;
      expect(input.validity.rangeOverflow).toBe(true);
    });
  });

  it("opens the native calendar when clicking anywhere on a date field", () => {
    document.body.innerHTML = `<section data-agent-memories><form data-memory-date-form><input type="date" name="start"></form></section>`;
    const input = document.querySelector<HTMLInputElement>('input[type="date"]')!;
    const showPicker = vi.fn();
    input.showPicker = showPicker;
    initMemoryTimelines();
    input.click();
    expect(showPicker).toHaveBeenCalledOnce();
  });
});
