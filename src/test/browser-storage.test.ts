// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";

afterEach(() => localStorage.clear());

it("shares jsdom storage between global and window access", () => {
  const dom = (globalThis as typeof globalThis & { jsdom: { window: Window } }).jsdom;
  expect(localStorage).toBe(dom.window.localStorage);
  localStorage.setItem("global-key", "global-value");
  expect(window.localStorage.getItem("global-key")).toBe("global-value");
  window.localStorage.setItem("window-key", "window-value");
  expect(localStorage.getItem("window-key")).toBe("window-value");
  localStorage.removeItem("global-key");
  expect(dom.window.localStorage.getItem("global-key")).toBeNull();
});
