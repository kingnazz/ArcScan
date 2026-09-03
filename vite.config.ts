import { readFileSync } from "node:fs";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @tauri-apps/cli sets TAURI_DEV_HOST when running `tauri dev`.
const host = process.env.TAURI_DEV_HOST;

// package.json is the single source of truth for the version. It is injected here
// so the UI, the Tauri bundle and the website all report the same number without
// anyone maintaining a second copy. `npm run sync-version` propagates it.
const pkg = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8")) as {
  version: string;
};
const buildSha = (process.env.ARCSCAN_BUILD_SHA ?? "").trim().slice(0, 7);

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
    __BUILD_SHA__: JSON.stringify(buildSha),
  },

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
    // Tauri uses Chromium on Windows and WebKit on macOS/Linux, so the bundle
    // can target whatever those ship.
    target: "esnext",
    // Vite 8 minifies with oxc and no longer bundles esbuild, so naming a
    // minifier explicitly only risks pinning us to one that is not installed.
    minify: true,
    sourcemap: false,
  },
}));
