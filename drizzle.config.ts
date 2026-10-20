import { defineConfig } from "drizzle-kit";

// Generation only: migrations are applied by the owning runtime, never this CLI.
export default defineConfig({
  dialect: "sqlite",
  schema: ["./src/db/auth-schema.ts", "./src/db/agent-schema.ts", "./src/db/exchange-schema.ts", "./src/db/provider-schema.ts", "./src/db/journal-schema.ts", "./src/db/order-schema.ts", "./src/db/subagent-schema.ts", "./src/db/artifact-schema.ts", "./src/db/memory-schema.ts", "./src/db/insight-schema.ts", "./src/db/execution-schema.ts", "./src/db/risk-schema.ts"],
  out: "./src/db/migrations",
});
