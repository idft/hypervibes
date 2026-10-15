import { defineConfig } from "drizzle-kit";

// Generation only: migrations are applied by the owning runtime, never this CLI.
export default defineConfig({
  dialect: "sqlite",
  schema: "./src/db/auth-schema.ts",
  out: "./src/db/migrations",
});
