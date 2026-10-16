import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { loadConfig } from "./config.js";
import { testAuthSecret, testConfig } from "./test-config.js";

describe("server config", () => {
  it("defaults to loopback and a fresh SQLite file, ignoring legacy database config", () => {
    const config = testConfig({ DATABASE_URL: "postgres://unused", APP_BIND_ADDR: "0.0.0.0:9999" });
    expect(config.host).toBe("127.0.0.1");
    expect(config.port).toBe(3003);
    expect(config.databasePath).toBe(resolve("data/hypervibes.sqlite"));
    expect(config.staticDir).toBe(resolve("static"));
    expect(config.buildDir).toBe(resolve(".build"));
    expect(config.authOrigin).toBe("http://localhost:3003");
  });

  it("accepts explicit paths and bind settings", () => {
    expect(testConfig({
      APP_HOST: " ::1 ",
      APP_PORT: "3103",
      APP_SQLITE_PATH: "private/test.sqlite",
      APP_STATIC_DIR: "public",
      APP_BUILD_DIR: "output",
    })).toEqual({
      authOrigin: "http://localhost:3103",
      authSecret: testAuthSecret,
      host: "::1",
      port: 3103,
      databasePath: resolve("private/test.sqlite"),
      staticDir: resolve("public"),
      buildDir: resolve("output"),
    });
  });

  it.each(["0", "65536", "-1", "3003.5", "3003oops", "", "Infinity"])("rejects invalid port %s", (port) => {
    expect(() => testConfig({ APP_PORT: port })).toThrow("APP_PORT");
  });

  it.each(["not-a-url", "", "http://example.com", "https://example.com/path", "https://user@example.com", "https://example.com/?q=1", "https://example.com/#fragment", "ftp://example.com"])("rejects unsafe auth origin %s", (origin) => {
    expect(() => testConfig({ APP_ORIGIN: origin })).toThrow("APP_ORIGIN");
  });

  it("accepts an HTTPS origin and rejects missing or short secrets", () => {
    expect(testConfig({ APP_ORIGIN: "https://app.example.com/" }).authOrigin).toBe("https://app.example.com");
    expect(() => loadConfig({})).toThrow("BETTER_AUTH_SECRET");
    expect(() => testConfig({ BETTER_AUTH_SECRET: "short" })).toThrow("BETTER_AUTH_SECRET");
  });

  it.each(["APP_HOST", "APP_SQLITE_PATH", "APP_STATIC_DIR", "APP_BUILD_DIR"])("rejects blank %s", (name) => {
    expect(() => testConfig({ [name]: " " })).toThrow(name);
  });

  it("parses the optional exchange encryption key", () => {
    const bytes = Buffer.alloc(32, 1);
    const hex = bytes.toString("hex");
    expect(testConfig({ AGENTS_ENCRYPTION_KEY: hex }).encryptionKey)
      .toEqual({ id: "primary", bytes: new Uint8Array(bytes) });
    expect(testConfig().encryptionKey).toBeUndefined();
    expect(() => testConfig({ AGENTS_ENCRYPTION_KEY: "not-hex" })).toThrow("valid hexadecimal");
    expect(() => testConfig({ AGENTS_ENCRYPTION_KEY: hex.slice(0, 62) })).toThrow("exactly 32 bytes");
  });
});
