import { defineConfig } from "vitest/config";

export default defineConfig({
  define: {
    // The version is normally injected by vite.config.ts; tests need it too.
    __APP_VERSION__: JSON.stringify("0.0.0-test"),
  },
  test: {
    // jsdom, because the preferences module talks to localStorage and the
    // keyboard helpers to window. Nothing here renders React.
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    restoreMocks: true,
  },
});
