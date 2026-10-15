import tailwindcss from "@tailwindcss/vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Only the new Start app uses Vite. Rust's existing pnpm build remains unchanged.
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
