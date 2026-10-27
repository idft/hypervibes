import type { BetterAuthOptions } from "better-auth";
import { APIError, createAuthMiddleware } from "better-auth/api";
import { siwe } from "better-auth/plugins";
import { randomBytes } from "node:crypto";
import { isAddress, isHex, verifyMessage } from "viem";
import { parseSiweMessage, validateSiweMessage } from "viem/siwe";
import type { AppConfig } from "./config.js";

export function createAuthOptions(config: Pick<AppConfig, "authOrigin" | "authSecret">) {
  const origin = new URL(config.authOrigin);
  const bindingCookie = `${origin.protocol === "https:" ? "__Host-" : ""}hv_siwe`;
  const bindingAttributes = { httpOnly: true, secure: origin.protocol === "https:", sameSite: "lax" as const, path: "/", maxAge: 300 };
  const isNoncePath = (path: string) => path === "/siwe/nonce" || path === "/siwe/get-nonce";

  return {
    appName: "HyperVibes",
    baseURL: config.authOrigin,
    secret: config.authSecret,
    trustedOrigins: [config.authOrigin],
    telemetry: { enabled: false },
    user: { modelName: "hvUsers" },
    account: { modelName: "hvAuthAccounts" },
    verification: { modelName: "hvAuthVerifications" },
    emailAndPassword: { enabled: false },
    databaseHooks: {
      verification: {
        create: {
          before: async (value) => value.identifier.startsWith("siwe:")
            ? { data: { ...value, expiresAt: new Date(Date.now() + 300_000) } }
            : undefined,
        },
      },
    },
    session: { modelName: "hvSessions", expiresIn: 60 * 60 * 24 * 7, cookieCache: { enabled: false } },
    advanced: {
      cookiePrefix: "hv_auth",
      // Follow the configured public origin, not NODE_ENV or proxy headers.
      useSecureCookies: origin.protocol === "https:",
      database: { generateId: "uuid" },
      trustedProxyHeaders: false,
      // The host overwrites this header with the socket IP; never trust client XFF.
      ipAddress: { ipAddressHeaders: ["x-hv-client-ip"] },
    },
    rateLimit: {
      enabled: true,
      window: 60,
      max: 100,
      customRules: { "/siwe/*": { window: 60, max: 20 } },
    },
    hooks: {
      before: createAuthMiddleware(async (ctx) => {
        if (ctx.request && ctx.request.method !== "GET" && ctx.request.headers.get("origin") !== config.authOrigin) {
          throw new APIError("FORBIDDEN", { message: "Invalid request origin" });
        }
        if (ctx.path === "/siwe/verify") {
          const message: unknown = ctx.body?.message;
          const nonce = typeof message === "string" ? parseSiweMessage(message).nonce : undefined;
          const binding = await ctx.getSignedCookie(bindingCookie, config.authSecret);
          if (!nonce || binding !== nonce) {
            throw new APIError("UNAUTHORIZED", { message: "Login expired. Please try again." });
          }
        }
      }),
      after: createAuthMiddleware(async (ctx) => {
        const result: unknown = ctx.context.returned;
        if (isNoncePath(ctx.path) && result && typeof result === "object" && "nonce" in result && typeof result.nonce === "string") {
          await ctx.setSignedCookie(bindingCookie, result.nonce, config.authSecret, bindingAttributes);
        }
        if (ctx.path === "/siwe/verify") ctx.setCookie(bindingCookie, "", { ...bindingAttributes, maxAge: 0 });
      }),
    },
    plugins: [siwe({
      domain: origin.host,
      schema: { walletAddress: { modelName: "hvWalletAddresses" } },
      anonymous: true, // Wallet-only sign-in. Better Auth supplies a placeholder email internally.
      getNonce: async () => randomBytes(32).toString("hex"),
      verifyMessage: async ({ message, signature, address, chainId }) => {
        try {
          const parsed = parseSiweMessage(message);
          const now = Date.now();
          const issuedAt = parsed.issuedAt?.getTime() ?? NaN;
          const expiresAt = parsed.expirationTime?.getTime() ?? NaN;
          // Lock this first slice to the legacy identity chain, exact login URI,
          // and a five-minute proof. This is not exchange/trading authorization.
          if (chainId !== 1 || parsed.uri !== `${config.authOrigin}/login` || parsed.version !== "1"
            || !Number.isFinite(issuedAt) || !Number.isFinite(expiresAt)
            || issuedAt > now || now - issuedAt > 300_000
            || expiresAt <= issuedAt || expiresAt - issuedAt > 300_000
            || !isAddress(address) || !isHex(signature)
            || !validateSiweMessage({ message: parsed, address, domain: origin.host })) return false;
          // Pure EOA verification: no RPC/ENS/exchange calls during login.
          return await verifyMessage({ address, message, signature });
        } catch {
          return false;
        }
      },
    })],
  } satisfies BetterAuthOptions;
}
