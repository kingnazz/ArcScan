import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  define: {
    // The version is normally injected by vite.config.ts; tests need it too.
    __APP_VERSION__: JSON.stringify("0.0.0-test"),
  },
  test: {
    // jsdom, because the preferences module talks to localStorage, the keyboard
    // helpers talk to window, and the public-IP hook is exercised through a
    // real React render.
    environment: "jsdom",
    // The packaging scripts get tests too: they decide what actually ships,
    // and "the ZIP contained the wrong architecture" is not something to find
    // out from a bug report.
    include: ["src/**/*.test.ts", "src/**/*.test.tsx", "scripts/**/*.test.mjs"],
    restoreMocks: true,
  },
});
