import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// Tauri serves the built files itself; the dev server is only for `tauri dev`
// and for the browser preview (which runs on fixture data, see src/lib/mock.ts).
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: "es2022", sourcemap: false, chunkSizeWarningLimit: 900 },
});
