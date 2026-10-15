import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { loadConfig } from "./config.js";

describe("server config", () => {
  it("defaults to loopback and a fresh SQLite file, ignoring legacy database config", () => {
    const config = loadConfig({ DATABASE_URL: "postgres://unused", APP_BIND_ADDR: "0.0.0.0:9999" });
    expect(config.host).toBe("127.0.0.1");
    expect(config.port).toBe(3003);
    expect(config.databasePath).toBe(resolve("data/hypervibes.sqlite"));
    expect(config.staticDir).toBe(resolve("static"));
    expect(config.buildDir).toBe(resolve(".build"));
  });

  it("accepts explicit paths and bind settings", () => {
    expect(loadConfig({
      APP_HOST: " ::1 ",
      APP_PORT: "3103",
      APP_SQLITE_PATH: "private/test.sqlite",
      APP_STATIC_DIR: "public",
      APP_BUILD_DIR: "output",
    })).toEqual({
      host: "::1",
      port: 3103,
      databasePath: resolve("private/test.sqlite"),
      staticDir: resolve("public"),
      buildDir: resolve("output"),
    });
  });

  it.each(["0", "65536", "-1", "3003.5", "3003oops", "", "Infinity"])("rejects invalid port %s", (port) => {
    expect(() => loadConfig({ APP_PORT: port })).toThrow("APP_PORT");
  });

  it.each(["APP_HOST", "APP_SQLITE_PATH", "APP_STATIC_DIR", "APP_BUILD_DIR"])("rejects blank %s", (name) => {
    expect(() => loadConfig({ [name]: " " })).toThrow(name);
  });
});
