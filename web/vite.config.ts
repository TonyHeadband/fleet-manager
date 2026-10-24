import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// `pnpm dev` talks to a daemon on the default port; a built UI is served by the daemon itself.
export default defineConfig({
  plugins: [react()],
  server: { proxy: { "/api": "http://127.0.0.1:7878" } },
  test: { environment: "jsdom", globals: true, setupFiles: ["src/test/setup.ts"] },
});
