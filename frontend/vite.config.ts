import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite config for the Tauri WebView. Fixed port so `devUrl` in tauri.conf.json
// resolves deterministically; Tauri owns the process lifecycle so we don't open
// a browser.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
});
