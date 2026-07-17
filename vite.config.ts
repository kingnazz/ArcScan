import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @tauri-apps/cli sets TAURI_DEV_HOST when running `tauri dev`.
const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // Prevent Vite from obscuring Rust errors.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // Tell Vite to ignore watching `src-tauri`.
      ignored: ["**/src-tauri/**"],
    },
  },

  // Env variables starting with VITE_ are exposed to the client.
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // Tauri uses Chromium on Windows and WebKit on macOS/Linux.
    target: "esnext",
    minify: "esbuild",
    sourcemap: false,
  },
}));
