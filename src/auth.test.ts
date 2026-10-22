import { eq, sql } from "drizzle-orm";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { privateKeyToAccount } from "viem/accounts";
import { createSiweMessage } from "viem/siwe";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { currentUser } from "./auth-user.js";
import { hvAuthVerifications, hvSessions } from "./db/auth-schema.js";
import { createRuntime } from "./runtime.js";
import type { AppRuntime } from "./runtime.js";
import { testConfig } from "./test-config.js";

// Local test-only EOA: no RPC, exchange credentials or real wallet is used.
const wallet = privateKeyToAccount(`0x${"11".repeat(32)}`);
const otherWallet = privateKeyToAccount(`0x${"22".repeat(32)}`);
const origin = "http://localhost:3003";
const cookies = (response: Response) => response.headers.getSetCookie()
  .filter((cookie) => !cookie.includes("Max-Age=0"))
  .map((cookie) => cookie.split(";")[0]).join("; ");
let ipSuffix = 0;

describe("Better Auth SIWE on Node SQLite", () => {
  let directory: string;
  let runtime: AppRuntime;
  let ip: string;
  const config = () => testConfig({ APP_SQLITE_PATH: join(directory, "test.sqlite") });
  const headers = (cookie = "", requestOrigin = origin) => new Headers({
    "content-type": "application/json", origin: requestOrigin, cookie, "x-hv-client-ip": ip,
  });
  const request = (path: string, body?: unknown, cookie = "", requestOrigin = origin) => runtime.auth.handler(new Request(`${origin}/api/auth${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: headers(cookie, requestOrigin),
    body: body === undefined ? undefined : JSON.stringify(body),
  }));
  async function challenge(overrides: Partial<Parameters<typeof createSiweMessage>[0]> = {}) {
    const response = await request("/siwe/nonce", {});
    expect(response.status).toBe(200);
    const { nonce } = await response.json() as { nonce: string };
    expect(nonce).toMatch(/^[a-zA-Z0-9]{8,250}$/);
    const now = new Date();
    const message = createSiweMessage({
      address: wallet.address, chainId: 1, domain: "localhost:3003", uri: `${origin}/login`,
      version: "1", nonce, issuedAt: now, expirationTime: new Date(now.getTime() + 300_000), ...overrides,
    });
    return { nonce, cookie: cookies(response), message, signature: await wallet.signMessage({ message }) };
  }
  async function login() {
    const proof = await challenge();
    const response = await request("/siwe/verify", { message: proof.message, signature: proof.signature }, proof.cookie);
    expect(response.status).toBe(200);
    return { response, cookie: cookies(response) };
  }
  beforeEach(async () => {
    directory = mkdtempSync(join(tmpdir(), "hypervibes-auth-"));
    ip = `127.0.0.${++ipSuffix}`;
    runtime = await createRuntime(config());
  });
  afterEach(async () => {
    await runtime.close();
    rmSync(directory, { recursive: true, force: true });
  });

  it("creates a wallet-only user and persistent session, exposing only public identity", async () => {
    const { cookie, response } = await login();
    const setCookie = response.headers.getSetCookie().find((value) => value.includes("hv_auth.session_token="));
    expect(setCookie).toContain("HttpOnly");
    expect(setCookie).toContain("SameSite=Lax");
    expect(setCookie).toContain("Path=/");
    expect(setCookie).toContain("Max-Age=604800");
    const identity = await currentUser(runtime, headers(cookie));
    expect(identity).toEqual({ id: expect.any(String), walletAddress: wallet.address });
    expect(await runtime.db.values((sql`SELECT provider_id FROM hv_auth_accounts`))).toEqual([["siwe"]]);
    await runtime.close();
    runtime = await createRuntime(config());
    expect(await currentUser(runtime, headers(cookie))).toEqual(identity);
    expect(await runtime.db.values((sql`SELECT * FROM hv_schema_migrations`))).toHaveLength(12);
  });

  it("reuses the same user on subsequent logins", async () => {
    const first = await login();
    const second = await login();
    expect(await currentUser(runtime, headers(first.cookie))).toEqual(await currentUser(runtime, headers(second.cookie)));
    expect(await runtime.db.values((sql`SELECT id FROM hv_users`))).toHaveLength(1);
    expect(await runtime.db.values((sql`SELECT id FROM hv_wallet_addresses`))).toHaveLength(1);
    expect(await runtime.db.values((sql`SELECT id FROM hv_auth_accounts`))).toHaveLength(1);
  });

  it("rejects missing, tampered, expired and revoked sessions", async () => {
    expect(await currentUser(runtime, headers())).toBeNull();
    const first = await login();
    expect(await currentUser(runtime, headers(first.cookie.replace("session_token=", "session_token=tampered-")))).toBeNull();
    await runtime.db.update(hvSessions).set({ expiresAt: new Date(Date.now() - 1_000) }).run();
    expect(await currentUser(runtime, headers(first.cookie))).toBeNull();
    const second = await login();
    const response = await request("/sign-out", {}, second.cookie);
    expect(response.status).toBe(200);
    expect(response.headers.getSetCookie().join(";")).toContain("Max-Age=0");
    expect(await currentUser(runtime, headers(second.cookie))).toBeNull();
    expect(await runtime.db.values((sql`SELECT id FROM hv_sessions`))).toEqual([]);
  });

  it("requires a same-origin POST for login and logout", async () => {
    for (const requestOrigin of ["https://evil.example", "null", ""]) {
      expect((await request("/siwe/nonce", {}, "", requestOrigin)).status).toBe(403);
    }
    const { cookie } = await login();
    expect((await request("/sign-out", {}, cookie, "https://evil.example")).status).toBe(403);
    expect(await currentUser(runtime, headers(cookie))).not.toBeNull();
    expect((await request("/sign-out", undefined, cookie)).status).not.toBe(200);
  });

  it("binds each proof to the initiating browser", async () => {
    const proof = await challenge();
    const body = { message: proof.message, signature: proof.signature };
    expect((await request("/siwe/verify", body)).status).toBe(401);
    const other = await challenge();
    expect((await request("/siwe/verify", body, other.cookie)).status).toBe(401);
    expect((await request("/siwe/verify", body, proof.cookie)).status).toBe(200);
  });

  it("consumes a nonce at most once under concurrent verification", async () => {
    const proof = await challenge();
    const body = { message: proof.message, signature: proof.signature };
    const responses = await Promise.all([request("/siwe/verify", body, proof.cookie), request("/siwe/verify", body, proof.cookie)]);
    expect(responses.map((response) => response.status).sort()).toEqual([200, 401]);
    expect((await request("/siwe/verify", body, proof.cookie)).status).toBe(401);
    expect(await runtime.db.values((sql`SELECT id FROM hv_sessions`))).toHaveLength(1);
  });

  it("rejects expired server nonces even with a freshly signed message", async () => {
    const proof = await challenge();
    const row = (await runtime.db.select().from(hvAuthVerifications).get())!;
    expect(row.expiresAt.getTime()).toBeLessThanOrEqual(Date.now() + 300_000);
    expect(row.expiresAt.getTime()).toBeGreaterThan(Date.now() + 299_000);
    await runtime.db.update(hvAuthVerifications).set({ expiresAt: new Date(Date.now() - 1_000) })
      .where(eq(hvAuthVerifications.identifier, `siwe:${proof.nonce}`)).run();
    expect((await request("/siwe/verify", { message: proof.message, signature: proof.signature }, proof.cookie)).status).toBe(401);
  });

  it.each([
    { domain: "evil.example" },
    { uri: "https://evil.example/login" },
    { chainId: 42161 },
    { expirationTime: undefined },
    { issuedAt: new Date(Date.now() - 600_000), expirationTime: new Date(Date.now() - 300_000) },
    { issuedAt: new Date(Date.now() + 600_000), expirationTime: new Date(Date.now() + 900_000) },
    { expirationTime: new Date(Date.now() + 3_600_000) },
  ])("rejects an invalid signed SIWE field: %j", async (overrides) => {
    const proof = await challenge(overrides);
    const response = await request("/siwe/verify", { message: proof.message, signature: proof.signature }, proof.cookie);
    expect(response.status).toBe(401);
    expect(await runtime.db.values((sql`SELECT id FROM hv_users`))).toEqual([]);
    expect(await runtime.db.values((sql`SELECT id FROM hv_sessions`))).toEqual([]);
  });

  it("rejects incorrect signatures, tampered messages and malformed payloads", async () => {
    const proof = await challenge();
    const wrongSignature = await otherWallet.signMessage({ message: proof.message });
    expect((await request("/siwe/verify", { message: proof.message, signature: wrongSignature }, proof.cookie)).status).toBe(401);
    const second = await challenge();
    expect((await request("/siwe/verify", { message: second.message.replace(wallet.address, otherWallet.address), signature: second.signature }, second.cookie)).status).toBe(401);
    expect((await request("/siwe/verify", { message: 123, signature: "invalid" })).status).toBe(401);
    expect(await runtime.db.values((sql`SELECT id FROM hv_users`))).toEqual([]);
  });

  it("bounds nonce issuance with Better Auth's rate limiter", async () => {
    for (let count = 0; count < 20; count++) expect((await request("/siwe/nonce", {})).status).toBe(200);
    expect((await request("/siwe/nonce", {})).status).toBe(429);
    expect(await runtime.db.values((sql`SELECT id FROM hv_auth_verifications`))).toHaveLength(20);
  });

  it("cleans expired sessions and verification records without deleting live state", async () => {
    const { cookie } = await login();
    const now = new Date();
    await runtime.db.insert(hvAuthVerifications).values(Array.from({ length: 120 }, (_, index) => ({
      id: `expired-${index}`, identifier: `expired-${index}`, value: "expired", createdAt: now, updatedAt: now, expiresAt: new Date(0),
    }))).run();
    await request("/siwe/nonce", {});
    expect(await runtime.db.values((sql`SELECT id FROM hv_auth_verifications WHERE expires_at = 0`))).toHaveLength(20);
    expect(await currentUser(runtime, headers(cookie))).not.toBeNull();
  });

  it("keeps password registration disabled", async () => {
    expect((await request("/sign-up/email", { email: "test@example.com", password: "unused-password", name: "test" })).status).not.toBe(200);
    expect(await runtime.db.values((sql`SELECT id FROM hv_users`))).toEqual([]);
  });

  it("uses Secure cookies on HTTPS without changing the trusted public origin", async () => {
    await runtime.close();
    runtime = await createRuntime({ ...config(), authOrigin: "https://app.example.com" });
    const response = await runtime.auth.handler(new Request("https://app.example.com/api/auth/siwe/nonce", {
      method: "POST", headers: { origin: "https://app.example.com", "content-type": "application/json", "x-hv-client-ip": ip }, body: "{}",
    }));
    expect(response.status).toBe(200);
    const cookie = response.headers.getSetCookie().join(";");
    expect(cookie).toContain("__Host-hv_siwe=");
    expect(cookie).toContain("Secure");
    expect(cookie).toContain("HttpOnly");
    expect(cookie).not.toContain("Domain=");
  });
});
