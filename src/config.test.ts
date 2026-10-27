import { join, resolve } from "node:path";
import { homedir } from "node:os";
import { describe, expect, it } from "vitest";
import { defaultCacheDirectory, defaultDatabasePath, loadConfig } from "./config.js";
import { testAuthSecret, testConfig } from "./test-config.js";

describe("server config", () => {
  it("defaults to loopback and an OS-appropriate per-user SQLite file, ignoring legacy database config", () => {
    const config = testConfig({ DATABASE_URL: "postgres://unused", APP_BIND_ADDR: "0.0.0.0:9999" });
    expect(config.host).toBe("127.0.0.1");
    expect(config.port).toBe(3003);
    const expectedDefault = process.platform === "win32"
      ? join(join(homedir(), "AppData", "Roaming"), "HyperVibes", "hypervibes.sqlite")
      : process.platform === "darwin"
        ? join(homedir(), "Library", "Application Support", "HyperVibes", "hypervibes.sqlite")
        : join(join(homedir(), ".local", "share"), "hypervibes", "hypervibes.sqlite");
    expect(config.databasePath).toBe(resolve(expectedDefault));
    expect(config.staticDir).toBe(resolve("static"));
    expect(config.buildDir).toBe(resolve(".build"));
    expect(config.cacheDir).toBe(resolve(defaultCacheDirectory({})));
    expect(config.authOrigin).toBe("http://localhost:3003");
  });

  it("uses each platform's per-user directory and absolute environment overrides", () => {
    expect(defaultDatabasePath({}, "linux", "/home/test")).toBe("/home/test/.local/share/hypervibes/hypervibes.sqlite");
    expect(defaultDatabasePath({ XDG_DATA_HOME: "/custom/data" }, "linux", "/home/test")).toBe("/custom/data/hypervibes/hypervibes.sqlite");
    expect(defaultDatabasePath({ XDG_DATA_HOME: "relative" }, "linux", "/home/test")).toBe("/home/test/.local/share/hypervibes/hypervibes.sqlite");
    expect(defaultDatabasePath({}, "darwin", "/Users/test")).toBe("/Users/test/Library/Application Support/HyperVibes/hypervibes.sqlite");
    expect(defaultDatabasePath({}, "win32", "C:\\Users\\test")).toBe("C:\\Users\\test\\AppData\\Roaming\\HyperVibes\\hypervibes.sqlite");
    expect(defaultDatabasePath({ APPDATA: "D:\\Profile" }, "win32", "C:\\Users\\test")).toBe("D:\\Profile\\HyperVibes\\hypervibes.sqlite");
    if (process.platform === "linux") expect(testConfig({ XDG_DATA_HOME: "/custom/data" }).databasePath).toBe("/custom/data/hypervibes/hypervibes.sqlite");
  });

  it("uses each platform's per-user cache directory", () => {
    expect(defaultCacheDirectory({}, "linux", "/home/test")).toBe("/home/test/.cache/hypervibes");
    expect(defaultCacheDirectory({ XDG_CACHE_HOME: " /custom/cache " }, "linux", "/home/test")).toBe("/custom/cache/hypervibes");
    for (const cacheHome of ["relative", "", " "]) {
      expect(defaultCacheDirectory({ XDG_CACHE_HOME: cacheHome }, "linux", "/home/test")).toBe("/home/test/.cache/hypervibes");
    }
    expect(defaultCacheDirectory({ XDG_CACHE_HOME: "/ignored" }, "darwin", "/Users/test")).toBe("/Users/test/Library/Caches/HyperVibes");
    expect(defaultCacheDirectory({}, "win32", "C:\\Users\\test")).toBe("C:\\Users\\test\\AppData\\Local\\HyperVibes\\Cache");
    expect(defaultCacheDirectory({ LOCALAPPDATA: " D:\\Profile " }, "win32", "C:\\Users\\test")).toBe("D:\\Profile\\HyperVibes\\Cache");
    for (const localAppData of ["relative", "C:relative", "", " "]) {
      expect(defaultCacheDirectory({ LOCALAPPDATA: localAppData }, "win32", "C:\\Users\\test")).toBe("C:\\Users\\test\\AppData\\Local\\HyperVibes\\Cache");
    }
    if (process.platform === "linux") expect(testConfig({ XDG_CACHE_HOME: "/custom/cache" }).cacheDir).toBe("/custom/cache/hypervibes");
  });

  it("preserves explicit cache directory overrides", () => {
    expect(testConfig({ APP_CACHE_DIR: " private/cache ", XDG_CACHE_HOME: "/ignored" }).cacheDir).toBe(resolve("private/cache"));
    expect(testConfig({ APP_CACHE_DIR: resolve("custom-cache") }).cacheDir).toBe(resolve("custom-cache"));
  });

  it("accepts explicit paths and bind settings", () => {
    expect(testConfig({
      APP_HOST: " ::1 ",
      APP_PORT: "3103",
      APP_SQLITE_PATH: "private/test.sqlite",
      APP_STATIC_DIR: "public",
      APP_BUILD_DIR: "output",
      APP_CACHE_DIR: "private/cache",
    })).toEqual({
      authOrigin: "http://localhost:3103",
      authSecret: testAuthSecret,
      host: "::1",
      port: 3103,
      databasePath: resolve("private/test.sqlite"),
      staticDir: resolve("public"),
      buildDir: resolve("output"),
      cacheDir: resolve("private/cache"),
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

  it.each(["APP_HOST", "APP_SQLITE_PATH", "APP_STATIC_DIR", "APP_BUILD_DIR", "APP_CACHE_DIR"])("rejects blank %s", (name) => {
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
