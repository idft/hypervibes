import { webcrypto } from "node:crypto";
import { afterEach, describe, expect, it, vi } from "vitest";
import { randomUuid } from "./random-uuid.js";

const uuidV4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

describe("browser randomUuid", () => {
  it("uses cryptographic bytes on HTTP without randomUUID and produces distinct IDs", () => {
    const getRandomValues = vi.fn((bytes: Uint8Array) => webcrypto.getRandomValues(bytes));
    vi.stubGlobal("crypto", { getRandomValues });
    const ids = Array.from({ length: 1000 }, () => randomUuid());
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids.every((id) => uuidV4.test(id))).toBe(true);
    expect(getRandomValues).toHaveBeenCalledTimes(ids.length);
    expect(getRandomValues.mock.calls.every(([bytes]) => bytes.length === 16)).toBe(true);
  });

  it.each([[0x00, "00000000-0000-4000-8000-000000000000"], [0x55, "55555555-5555-4555-9555-555555555555"], [0xaa, "aaaaaaaa-aaaa-4aaa-aaaa-aaaaaaaaaaaa"], [0xff, "ffffffff-ffff-4fff-bfff-ffffffffffff"]] as const)("sets version and variant bits for byte %i", (byte, expected) => {
    vi.stubGlobal("crypto", { getRandomValues: (bytes: Uint8Array) => bytes.fill(byte) });
    expect(randomUuid()).toBe(expected);
  });

  it("uses the native UUID method with its crypto receiver when available", () => {
    const crypto = { randomUUID: vi.fn(function (this: unknown) { expect(this).toBe(crypto); return "native-uuid"; }), getRandomValues: vi.fn() };
    vi.stubGlobal("crypto", crypto);
    expect(randomUuid()).toBe("native-uuid");
    expect(crypto.getRandomValues).not.toHaveBeenCalled();
  });

  it.each([undefined, {}])("fails clearly when Web Crypto is unavailable: %j", (crypto) => {
    vi.stubGlobal("crypto", crypto);
    expect(() => randomUuid()).toThrow("Web Crypto is unavailable. Cannot generate a secure UUID.");
  });

  it("propagates random byte failures without an insecure fallback", () => {
    vi.stubGlobal("crypto", { getRandomValues: () => { throw new Error("Random bytes unavailable"); } });
    expect(() => randomUuid()).toThrow("Random bytes unavailable");
  });

  it("does not access Web Crypto on import", async () => {
    vi.resetModules();
    const access = vi.spyOn(globalThis, "crypto", "get").mockImplementation(() => { throw new Error("Unexpected crypto access"); });
    await expect(import("./random-uuid.js")).resolves.toHaveProperty("randomUuid");
    expect(access).not.toHaveBeenCalled();
  });
});
