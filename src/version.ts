// The application version and build identity.
//
// The version is injected at build time from package.json by vite.config.ts, so
// there is exactly one place the version is written. `scripts/sync-version.mjs`
// propagates that same value to Cargo.toml, the Tauri config and the website,
// and CI fails if any of them drift.
//
// CI/release builds may also inject the source commit SHA. It is diagnostic only
// and never changes update/version comparison semantics.

declare const __APP_VERSION__: string;
declare const __BUILD_SHA__: string;

export const APP_VERSION: string =
  typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "0.0.0-dev";

export const BUILD_SHA: string =
  typeof __BUILD_SHA__ === "string" ? __BUILD_SHA__.trim().slice(0, 7) : "";

export const APP_BUILD_LABEL = `v${APP_VERSION}${BUILD_SHA ? ` · ${BUILD_SHA}` : ""}`;
