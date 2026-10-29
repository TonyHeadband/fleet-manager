import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Node globals this file uses, declared here instead of depending on @types/node for the dev config alone.
declare const process: { env: Record<string, string | undefined> };

// `pnpm dev` talks to a daemon on the default port (FLEET_DAEMON_URL overrides it); a built UI is served by the
// daemon itself.
const daemon = process.env.FLEET_DAEMON_URL ?? "http://127.0.0.1:7878";

// The one proxy event used here. Vite types the proxy as a Node EventEmitter, which needs @types/node as well.
type ProxyEvents = {
  on(
    event: "proxyReq",
    listener: (proxyReq: { setHeader(name: string, value: string): void }, req: { headers: Record<string, string | string[] | undefined> }) => void,
  ): void;
};

export default defineConfig({
  plugins: [react()],
  server: {
    // The fonts are the egui app's own files (see src/generated/theme.css), outside this project.
    fs: { allow: [".", "../crates/fleet-gui/assets/fonts"] },
    proxy: {
      "/api": {
        target: daemon,
        // The daemon refuses a Host or an Origin that is not its own (another web page driving the API). The dev
        // server is another origin, so its own page's requests are made to look same-origin; any other page's
        // Origin passes through unchanged and is still refused.
        changeOrigin: true,
        configure: (proxy) => {
          (proxy as unknown as ProxyEvents).on("proxyReq", (proxyReq, req) => {
            if (req.headers.origin === `http://${req.headers.host}`) {
              proxyReq.setHeader("origin", daemon);
            }
          });
        },
      },
    },
  },
  test: { environment: "jsdom", globals: true, setupFiles: ["src/test/setup.ts"] },
});
