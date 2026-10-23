import tailwindcss from "@tailwindcss/vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The application is a TanStack Start + Vite build served by src/web/app.ts.
export default defineConfig({
  publicDir: false, // The application host serves the existing /static/ directory.
  plugins: [tanstackStart(), tailwindcss(), react()],
  build: { outDir: ".build" },
  environments: {
    ssr: {
      build: { rolldownOptions: { output: { entryFileNames: "server.js" } } },
    },
  },
});
