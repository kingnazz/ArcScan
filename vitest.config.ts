import { defineConfig } from "vitest/config";

export default defineConfig({
  define: {
    // The version is normally injected by vite.config.ts; tests need it too.
    __APP_VERSION__: JSON.stringify("0.0.0-test"),
  },
  test: {
    // jsdom, because the preferences module talks to localStorage, the keyboard
    // helpers talk to window, and the public-IP hook is exercised through a
    // real React render.
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    restoreMocks: true,
  },
});
