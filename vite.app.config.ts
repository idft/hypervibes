import tailwindcss from "@tailwindcss/vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { buildLabel } from "./src/build/build-label.js";

// The application is a TanStack Start + Vite build served by src/web/app.ts.
export default defineConfig(({ command }) => ({
  define: { "import.meta.env.VITE_APP_BUILD_LABEL": JSON.stringify(buildLabel(command)) },
  publicDir: false, // The application host serves the existing /static/ directory.
  plugins: [tanstackStart(), tailwindcss(), react()],
  build: { outDir: ".build" },
  environments: {
    ssr: {
      build: { rolldownOptions: { output: { entryFileNames: "server.js" } } },
    },
  },
}));
