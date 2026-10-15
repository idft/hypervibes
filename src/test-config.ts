import { loadConfig } from "./config.js";

// Public, deterministic test fixture. Never use a real application's secret/database.
export const testAuthSecret = "test-only-auth-secret-97c14b2ea093f865d471";

export function testConfig(env: NodeJS.ProcessEnv = {}) {
  return loadConfig({ BETTER_AUTH_SECRET: testAuthSecret, ...env });
}
