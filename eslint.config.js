import js from "@eslint/js";
import tseslint from "typescript-eslint";
import globals from "globals";

export default tseslint.config(
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    languageOptions: {
      globals: {
        ...globals.browser,
      },
    },
  },
  {
    files: ["src/**/*.{ts,tsx}", "vite.app.config.ts", "vitest.server.config.ts"],
    languageOptions: {
      globals: { ...globals.node },
    },
  },
  {
    ignores: ["node_modules/", ".build/", "src/routeTree.gen.ts"],
  }
);
