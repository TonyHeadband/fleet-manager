import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { FleetClient } from "./api/client";
import { detectNative } from "./native/native";
import "./styles.css";

declare global {
  interface Window {
    /** Fleet endpoint override; empty means the daemon that served this page. */
    __FLEET_URL__?: string;
  }
}

// In the Tauri app the page comes from the app, not the daemon, so the daemon's address comes from the bridge.
async function start() {
  const native = await detectNative();
  const base = native ? await native.daemonUrl() : (window.__FLEET_URL__ ?? import.meta.env.VITE_FLEET_URL ?? "");
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App client={new FleetClient(base)} native={native} />
    </StrictMode>,
  );
}

void start();
