// The application version.
//
// Injected at build time from package.json by vite.config.ts, so there is exactly
// one place the version is written. `scripts/sync-version.mjs` propagates that
// same value to Cargo.toml, the Tauri config and the website, and CI fails if any
// of them drift.

declare const __APP_VERSION__: string;

export const APP_VERSION: string =
  typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "0.0.0-dev";
