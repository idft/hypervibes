import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const containerfile = readFileSync(new URL("../Containerfile", import.meta.url), "utf8");
const line = containerfile.split("\n").find((value) => value.trim().startsWith("CMD ["));
const [command, flag, script] = JSON.parse(line.trim().slice(4));

function exercise(env, statusCode = 200) {
  const exits = [];
  let options;
  const request = new EventEmitter();
  request.destroy = (error) => request.emit("error", error);
  const http = { get(value, callback) {
    options = value;
    callback({ statusCode, resume() {} });
    return request;
  } };
  runInNewContext(script, {
    URL,
    process: { env, exit(code) { exits.push(code); } },
    require(name) { assert.equal(name, "node:http"); return http; },
  });
  return { options, request, exits };
}

test("container health check connects through loopback with the configured Host", () => {
  assert.equal(command, "node");
  assert.equal(flag, "-e");
  for (const env of [{}, { APP_ORIGIN: "https://example.com" }, { APP_ORIGIN: "https://example.com:8443", APP_PORT: "4000" }]) {
    const { options, exits } = exercise(env);
    assert.equal(options.hostname, "127.0.0.1");
    assert.equal(options.port, env.APP_PORT || "3003");
    assert.equal(options.path, "/healthz");
    assert.equal(options.headers.host, new URL(env.APP_ORIGIN || "http://localhost:3003").host);
    assert.equal(options.timeout, 4000);
    assert.deepEqual(exits, []);
  }
});

test("container health check fails on HTTP errors, socket errors, and timeouts", () => {
  assert.deepEqual(exercise({}, 503).exits, [1]);
  for (const event of ["error", "timeout"]) {
    const { request, exits } = exercise({});
    request.emit(event, new Error("test failure"));
    assert.deepEqual(exits, [1]);
  }
});
