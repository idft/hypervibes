import { and, eq } from "drizzle-orm";
import { hvWalletAddresses } from "./db/auth-schema.js";
import type { AppRuntime } from "./runtime.js";

export interface AuthenticatedUser {
  id: string;
  walletAddress: string;
}

/** Public identity and expiry only; never expose Better Auth's session token. */
export async function currentUserSession(runtime: AppRuntime, headers: Headers) {
  const session = await runtime.auth.api.getSession({ headers });
  if (!session) return null;
  const wallet = await runtime.db.select({ address: hvWalletAddresses.address }).from(hvWalletAddresses)
    .where(and(eq(hvWalletAddresses.userId, session.user.id), eq(hvWalletAddresses.chainId, 1))).get();
  return wallet ? { user: { id: session.user.id, walletAddress: wallet.address }, expiresAt: session.session.expiresAt } : null;
}

/** Returns only public identity, never Better Auth's bearer session token. */
export async function currentUser(runtime: AppRuntime, headers: Headers): Promise<AuthenticatedUser | null> {
  return (await currentUserSession(runtime, headers))?.user ?? null;
}
