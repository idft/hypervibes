import { defineConfig } from "drizzle-kit";

// Generation only: migrations are applied by the owning runtime, never this CLI.
export default defineConfig({
  dialect: "sqlite",
  schema: ["./src/db/auth-schema.ts", "./src/db/agent-schema.ts", "./src/db/exchange-schema.ts", "./src/db/provider-schema.ts"],
  out: "./src/db/migrations",
});
